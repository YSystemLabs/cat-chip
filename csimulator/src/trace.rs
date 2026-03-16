//! Trace 导出：JSON trace 与人类可读报告
//!
//! 对应选型文档 §5.2 trace.rs。

use crate::report::CostReport;
use crate::schedule::Schedule;
use crate::simulate::CycleSnapshot;

/// 导出 Schedule 为 JSON 字符串
pub fn schedule_to_json(schedule: &Schedule) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(schedule)
}

/// 导出 CostReport 为 JSON 字符串
pub fn report_to_json(report: &CostReport) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(report)
}

/// 导出逐周期模拟轨迹为 JSON 字符串
///
/// 每个快照同时给出：
/// - end_of_cycle_occupancy: 周期结束时的缓冲占用
/// - instant_peak_occupancy: 周期内部的瞬时峰值占用
pub fn sim_trace_to_json(trace: &[CycleSnapshot]) -> Result<String, serde_json::Error> {
  serde_json::to_string_pretty(trace)
}

/// 人类可读的代价报告摘要
pub fn report_summary(report: &CostReport) -> String {
    format!(
        "\
=== CostReport ===
Structure: N_nz={}, N_gen={}, N_scale={}, N_unique={}, N_scale_u={}
           bcast_cols={}, reduce_rows={}
           dead_cols={}, dead_rows={}, single_rows={}, permutation={}

Analytic (§6.3 upper bound):
  C_load      = {} cycles
  C_load_eff  = {} cycles (double-buffered)
  C_exec      = {} cycles
  C_bcast     = {} cycles
  C_reduce    = {} cycles
  C_wb        = {} cycles
  C_total_up  = {} cycles
  C_total_eff = {} cycles

Scheduled (actual):
  load span   = {} cycles
  exec span   = {} cycles
  bcast span  = {} cycles
  reduce span = {} cycles
  wb span     = {} cycles
  total       = {} cycles

Resources:
  peak input  = {} blocks
  peak temp   = {} blocks
  output buf  = {} blocks
  peak output = {} blocks
  max core    = {}
  max bcast   = {}
  max reduce  = {}

Flat baseline:
  C_flat      = {} cycles
  R = block/flat = {:.4}
==================",
        report.n_nz,
        report.n_gen,
        report.n_scale,
        report.n_unique,
        report.n_scale_u,
        report.bcast_cols,
        report.reduce_rows,
        report.dead_cols,
        report.dead_rows,
        report.single_rows,
        report.is_permutation,
        report.analytic_load_cycles,
        report.analytic_load_eff_cycles,
        report.analytic_exec_cycles,
        report.analytic_bcast_cycles,
        report.analytic_reduce_cycles,
        report.analytic_wb_cycles,
        report.analytic_total_upper,
        report.analytic_total_eff,
        report.sched_load_span_cycles,
        report.sched_exec_span_cycles,
        report.sched_bcast_span_cycles,
        report.sched_reduce_span_cycles,
        report.sched_writeback_span_cycles,
        report.sched_total_cycles,
        report.peak_input_buffer_blocks,
        report.peak_temp_buffer_blocks,
        report.output_buffer_blocks,
        report.peak_output_buffer_blocks,
        report.max_active_core,
        report.max_active_bcast,
        report.max_active_reduce,
        report.flat_total,
        report.cost_ratio,
    )
}
