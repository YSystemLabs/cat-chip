//! 报告层：CostReport 与解析公式计算
//!
//! 对应规格 §6.3 代价公式与 §6.5 代价对比框架。
//! 同时输出解析上界值（来自公式）和实际调度值（来自 schedule_graph）。

use crate::ir::{BlockMat, D};
use crate::lower::SchedGraph;
use crate::optimize::StructStats;
use crate::schedule::{latency, Schedule, SchedParams};
use crate::simulate::SimResult;
use serde::Serialize;

/// 代价报告——与规格 §6.3 字段对齐
#[derive(Debug, Clone, Serialize)]
pub struct CostReport {
    // ---- 结构统计 ----
    pub n_nz: usize,
    pub n_gen: usize,
    pub n_scale: usize,
    pub n_unique: usize,
    pub n_scale_u: usize,
    pub bcast_cols: usize,
    pub reduce_rows: usize,
    pub dead_cols: usize,
    pub dead_rows: usize,
    pub single_rows: usize,
    pub is_permutation: bool,

    // ---- 解析代价（§6.3 公式） ----
    pub analytic_load_cycles: usize,
    pub analytic_load_eff_cycles: usize,
    pub analytic_exec_cycles: usize,
    pub analytic_bcast_cycles: usize,
    pub analytic_reduce_cycles: usize,
    pub analytic_wb_cycles: usize,
    pub analytic_total_upper: usize,
    pub analytic_total_eff: usize,

    // ---- 实际调度代价（schedule_graph 输出）----
    pub sched_exec_span_cycles: usize,
    pub sched_load_span_cycles: usize,
    pub sched_bcast_span_cycles: usize,
    pub sched_reduce_span_cycles: usize,
    pub sched_writeback_span_cycles: usize,
    pub sched_total_cycles: usize,

    // ---- 资源使用 ----
    pub peak_input_buffer_blocks: usize,
    pub peak_temp_buffer_blocks: usize,
    pub output_buffer_blocks: usize,
    pub peak_output_buffer_blocks: usize,
    pub max_active_core: usize,
    pub max_active_bcast: usize,
    pub max_active_reduce: usize,

    // ---- 展平路线代价（§6.5）----
    pub flat_total: usize,
    /// 代价比 R = block / flat
    pub cost_ratio: f64,
}

/// 从 StructStats + Schedule + SchedParams 生成报告
pub fn report_from_schedule(
    bm: &BlockMat,
    stats: &StructStats,
    graph: &SchedGraph,
    schedule: &Schedule,
    sim_result: &SimResult,
    params: &SchedParams,
) -> CostReport {
    let n = bm.rows;
    let m = bm.cols;

    // ---- 解析公式 ----

    // C_load = ceil(((N_unique - N_scale_u) * d^2 + N_scale_u * 1) / (P * B_w))
    let load_numerator =
        (stats.n_unique - stats.n_scale_u) * D * D + stats.n_scale_u;
    let load_denom = params.core_slots * params.bw_params;
    let c_load = ceil_div(load_numerator, load_denom);

    // C_exec = ceil((N_gen - N_scale)/P) * L_core + ceil(N_scale/P) * L_scale
    let general_count = stats.n_gen - stats.n_scale;
    let c_exec = ceil_div(general_count, params.core_slots) * params.core_latency
        + ceil_div(stats.n_scale, params.core_slots) * params.scale_latency;

    // C_bcast = ceil(|J_bcast| / B_ports) * max(C_bcast,j)
    let j_bcast: Vec<usize> = stats
        .col_counts
        .iter()
        .enumerate()
        .filter(|(_, &c)| c > 1)
        .map(|(j, _)| j)
        .collect();
    let max_bcast_lat = j_bcast
        .iter()
        .map(|&j| latency(&params.bcast_model, stats.col_counts[j]))
        .max()
        .unwrap_or(0);
    let c_bcast = ceil_div(j_bcast.len(), params.bcast_ports) * max_bcast_lat;

    // C_reduce = ceil(|I_reduce| / R_ports) * max(C_reduce,i)
    let i_reduce: Vec<usize> = stats
        .row_counts
        .iter()
        .enumerate()
        .filter(|(_, &r)| r > 1)
        .map(|(i, _)| i)
        .collect();
    let max_reduce_lat = i_reduce
        .iter()
        .map(|&i| latency(&params.reduce_model, stats.row_counts[i]))
        .max()
        .unwrap_or(0);
    let c_reduce = ceil_div(i_reduce.len(), params.reduce_ports) * max_reduce_lat;

    // C_wb = ceil(n * d / W_out)
    let c_wb = ceil_div(n * D, params.bw_writeback);

    // C_load_eff (双缓冲)
    let c_load_eff = if stats.n_gen == 0 {
        0
    } else {
        let n_batches = ceil_div(general_count, params.core_slots)
            + ceil_div(stats.n_scale, params.core_slots);
        let l_batch = params.scale_latency; // 保守取最小批次延迟
        let hiding = if n_batches > 1 {
            (n_batches - 1) * l_batch
        } else {
            0
        };
        c_load.saturating_sub(hiding)
    };

    let analytic_total_upper = c_load + c_exec + c_bcast + c_reduce + c_wb;
    let analytic_total_eff = c_load_eff + c_exec + c_bcast + c_reduce + c_wb;

    // ---- 展平路线 ----
    let nm = n * m;
    let flat_load = ceil_div(nm * D * D, params.core_slots * params.bw_params);
    let flat_exec = ceil_div(nm, params.core_slots) * params.core_latency;
    let flat_bcast = ceil_div(m, params.bcast_ports)
        * latency(&params.bcast_model, n);
    let flat_reduce = ceil_div(n, params.reduce_ports)
        * latency(&params.reduce_model, m);
    let flat_wb = ceil_div(n * D, params.bw_writeback);
    let flat_total = flat_load + flat_exec + flat_bcast + flat_reduce + flat_wb;

    let cost_ratio = if flat_total > 0 {
        analytic_total_eff as f64 / flat_total as f64
    } else {
        1.0
    };

    let sched_exec_span_cycles = class_span(graph, schedule, |nt| {
        matches!(nt, crate::lower::NodeType::Core(_))
    });
    let sched_load_span_cycles = class_span(graph, schedule, |nt| {
        matches!(nt, crate::lower::NodeType::InputRead | crate::lower::NodeType::ParamLoad)
    });
    let sched_bcast_span_cycles = class_span(graph, schedule, |nt| {
        matches!(nt, crate::lower::NodeType::Broadcast)
    });
    let sched_reduce_span_cycles = class_span(graph, schedule, |nt| {
        matches!(nt, crate::lower::NodeType::Reduce | crate::lower::NodeType::ZeroFill)
    });
    let sched_writeback_span_cycles = class_span(graph, schedule, |nt| {
        matches!(nt, crate::lower::NodeType::WriteBack)
    });

    let max_active_core = max_active_by_class(graph, schedule, |nt| {
        matches!(nt, crate::lower::NodeType::Core(_))
    });
    let max_active_bcast = max_active_by_class(graph, schedule, |nt| {
        matches!(nt, crate::lower::NodeType::Broadcast)
    });
    let max_active_reduce = max_active_by_class(graph, schedule, |nt| {
        matches!(nt, crate::lower::NodeType::Reduce | crate::lower::NodeType::ZeroFill)
    });

    CostReport {
        n_nz: stats.n_nz,
        n_gen: stats.n_gen,
        n_scale: stats.n_scale,
        n_unique: stats.n_unique,
        n_scale_u: stats.n_scale_u,
        bcast_cols: j_bcast.len(),
        reduce_rows: i_reduce.len(),
        dead_cols: stats.dead_cols,
        dead_rows: stats.dead_rows,
        single_rows: stats.single_rows,
        is_permutation: stats.is_permutation,
        analytic_load_cycles: c_load,
        analytic_load_eff_cycles: c_load_eff,
        analytic_exec_cycles: c_exec,
        analytic_bcast_cycles: c_bcast,
        analytic_reduce_cycles: c_reduce,
        analytic_wb_cycles: c_wb,
        analytic_total_upper,
        analytic_total_eff,
        sched_exec_span_cycles,
        sched_load_span_cycles,
        sched_bcast_span_cycles,
        sched_reduce_span_cycles,
        sched_writeback_span_cycles,
        sched_total_cycles: schedule.total_cycles,
        peak_input_buffer_blocks: sim_result.peak_input_buffer_blocks,
        peak_temp_buffer_blocks: sim_result.peak_temp_buffer,
        output_buffer_blocks: sim_result.output_buffer_blocks,
        peak_output_buffer_blocks: sim_result.peak_output_buffer_blocks,
        max_active_core,
        max_active_bcast,
        max_active_reduce,
        flat_total,
        cost_ratio,
    }
}

fn class_span<F>(graph: &SchedGraph, schedule: &Schedule, predicate: F) -> usize
where
    F: Fn(&crate::lower::NodeType) -> bool,
{
    let selected: Vec<_> = schedule
        .timed_nodes
        .iter()
        .filter(|timed| {
            graph.nodes
                .get(&timed.id)
                .map(|node| predicate(&node.node_type))
                .unwrap_or(false)
        })
        .collect();

    if selected.is_empty() {
        0
    } else {
        let start = selected.iter().map(|timed| timed.start_cycle).min().unwrap_or(0);
        let finish = selected.iter().map(|timed| timed.finish_cycle).max().unwrap_or(0);
        finish.saturating_sub(start)
    }
}

fn max_active_by_class<F>(graph: &SchedGraph, schedule: &Schedule, predicate: F) -> usize
where
    F: Fn(&crate::lower::NodeType) -> bool,
{
    (0..=schedule.total_cycles)
        .map(|cycle| {
            schedule
                .timed_nodes
                .iter()
                .filter(|timed| timed.start_cycle <= cycle && cycle < timed.finish_cycle)
                .filter(|timed| {
                    graph.nodes
                        .get(&timed.id)
                        .map(|node| predicate(&node.node_type))
                        .unwrap_or(false)
                })
                .count()
        })
        .max()
        .unwrap_or(0)
}

fn ceil_div(a: usize, b: usize) -> usize {
    if b == 0 {
        return 0;
    }
    (a + b - 1) / b
}
