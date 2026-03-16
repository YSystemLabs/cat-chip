//! 模拟层：在调度时间表上推进数值状态与缓冲区状态
//!
//! schedule.rs 是"谁在什么时候运行"；
//! simulate.rs 是"这些节点对数据和缓冲区做了什么"。

use crate::ir::D;
use crate::lower::{ExecKernelRef, NodeId, NodeType, SchedGraph};
use crate::schedule::{Schedule, SchedParams};
use crate::semantics::mat_vec_mul;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

// ============================================================
// 资源状态
// ============================================================

#[derive(Debug, Clone)]
pub enum SlotState {
    Idle,
    Busy { node: NodeId, until: usize },
}

/// 机器状态——内部缓冲区以块数为单位追踪
#[derive(Debug, Clone)]
pub struct MachineState {
    pub cycle: usize,
    pub core_slots: Vec<SlotState>,
    pub bcast_ports: Vec<SlotState>,
    pub reduce_ports: Vec<SlotState>,
    /// 当前输入缓冲占用（块数）
    pub input_buffer_occupancy: usize,
    /// 当前临时缓冲占用（块数）
    pub temp_buffer_occupancy: usize,
    /// 当前输出缓冲占用（块数）
    pub output_buffer_occupancy: usize,
    /// 临时缓冲峰值占用（块数）
    pub peak_temp_buffer: usize,
}

impl MachineState {
    pub fn new(params: &SchedParams) -> Self {
        MachineState {
            cycle: 0,
            core_slots: vec![SlotState::Idle; params.core_slots],
            bcast_ports: vec![SlotState::Idle; params.bcast_ports],
            reduce_ports: vec![SlotState::Idle; params.reduce_ports],
            input_buffer_occupancy: 0,
            temp_buffer_occupancy: 0,
            output_buffer_occupancy: 0,
            peak_temp_buffer: 0,
        }
    }

    pub fn track_temp_alloc(&mut self) {
        self.temp_buffer_occupancy += 1;
        if self.temp_buffer_occupancy > self.peak_temp_buffer {
            self.peak_temp_buffer = self.temp_buffer_occupancy;
        }
    }

    pub fn track_temp_free(&mut self) {
        self.temp_buffer_occupancy = self.temp_buffer_occupancy.saturating_sub(1);
    }
}

// ============================================================
// 逐周期快照（可选，用于 trace）
// ============================================================

#[derive(Debug, Clone, Serialize)]
pub struct CycleSnapshot {
    pub cycle: usize,
    pub active_nodes: Vec<NodeId>,
    pub input_buffer_occupancy: usize,
    /// 该周期内处理所有完成/释放动作之后的期末占用
    pub end_of_cycle_occupancy: usize,
    /// 该周期内任意时刻观察到的瞬时峰值占用
    pub instant_peak_occupancy: usize,
    pub output_buffer_occupancy: usize,
}

// ============================================================
// 模拟结果
// ============================================================

/// simulate.rs 的输出类型
#[derive(Debug, Clone)]
pub struct SimResult {
    /// n 个 d 维输出块
    pub output_vectors: Vec<[f32; D]>,
    /// 临时缓冲峰值（块数）
    pub peak_temp_buffer: usize,
    /// 逐周期轨迹（可选）
    pub per_cycle_trace: Vec<CycleSnapshot>,
    /// 输入缓冲峰值占用块数
    pub peak_input_buffer_blocks: usize,
    /// 输出缓冲占用块数
    pub output_buffer_blocks: usize,
    /// 输出缓冲峰值占用块数
    pub peak_output_buffer_blocks: usize,
}

// ============================================================
// 模拟执行
// ============================================================

/// 在已有 Schedule 上执行数值模拟
///
/// `input_blocks`: m 个 d 维输入块
pub fn simulate(
    graph: &SchedGraph,
    schedule: &Schedule,
    _params: &SchedParams,
    input_blocks: &[[f32; D]],
) -> SimResult {
    // 缓冲区：NodeId → d 维向量
    let mut buffers: HashMap<NodeId, [f32; D]> = HashMap::new();
    // 输入块映射（列索引 → 数据）：简化版，假设 input_slots 编码列索引
    let mut input_map: HashMap<usize, [f32; D]> = HashMap::new();
    for (j, block) in input_blocks.iter().enumerate() {
        input_map.insert(j, *block);
    }

    let mut output_vectors: Vec<[f32; D]> = vec![[0.0; D]; graph.output_nodes.len()];
    let output_node_set: HashSet<NodeId> = graph.output_nodes.iter().copied().collect();
    let topo_pos: HashMap<NodeId, usize> = schedule
        .topo_order
        .iter()
        .enumerate()
        .map(|(pos, &nid)| (nid, pos))
        .collect();
    let mut finish_map: HashMap<usize, Vec<NodeId>> = HashMap::new();
    for timed in &schedule.timed_nodes {
        finish_map
            .entry(timed.finish_cycle)
            .or_default()
            .push(timed.id);
    }
    for node_ids in finish_map.values_mut() {
        node_ids.sort_by_key(|nid| topo_pos.get(nid).copied().unwrap_or(usize::MAX));
    }

    let mut remaining_consumers: HashMap<NodeId, usize> =
        graph.nodes.keys().map(|&nid| (nid, 0usize)).collect();
    for node in graph.nodes.values() {
        for &dep in &node.deps {
            *remaining_consumers.entry(dep).or_insert(0) += 1;
        }
    }

    let mut peak_temp = 0usize;
    let mut temp_count = 0usize;
    let mut peak_input_buffer = 0usize;
    let mut current_input_buffer = 0usize;
    let mut peak_output_buffer = 0usize;
    let mut current_output_buffer = 0usize;

    let mut remaining_input_consumers: HashMap<usize, usize> = HashMap::new();
    let mut live_input_cols = HashSet::new();
    for node in graph.nodes.values() {
        if let Some(&addr) = node.input_slots.first() {
            if is_input_addr(addr) {
                live_input_cols.insert(addr);
                if matches!(node.node_type, NodeType::Broadcast) || node.deps.is_empty() {
                    *remaining_input_consumers.entry(addr).or_insert(0) += 1;
                }
            }
        }
    }

    let mut per_cycle_trace = Vec::with_capacity(schedule.total_cycles.saturating_add(1));

    for cycle in 0..=schedule.total_cycles {
        let mut cycle_peak = temp_count;
        if let Some(node_ids) = finish_map.get(&cycle) {
            for &nid in node_ids {
                let node = &graph.nodes[&nid];
                match &node.node_type {
                    NodeType::InputRead => {
                        current_input_buffer = live_input_cols.len();
                        peak_input_buffer = peak_input_buffer.max(current_input_buffer);
                    }
                    NodeType::ParamLoad => {}
                    NodeType::Broadcast => {
                        if let Some(&addr) = node.input_slots.first() {
                            if let Some(&data) = input_map.get(&addr) {
                                allocate_temp(
                                    &mut buffers,
                                    nid,
                                    data,
                                    &mut temp_count,
                                    &mut peak_temp,
                                    &mut cycle_peak,
                                );
                            }
                        }
                    }
                    NodeType::Core(exec_ref) => {
                        let input_data = resolve_input(node, &buffers, &input_map);
                        let output = match exec_ref {
                            ExecKernelRef::General(m) => mat_vec_mul(m, &input_data),
                            ExecKernelRef::Scalar(c) => {
                                let mut out = [0.0; D];
                                for i in 0..D {
                                    out[i] = c * input_data[i];
                                }
                                out
                            }
                            ExecKernelRef::Id => input_data,
                            ExecKernelRef::Zero => [0.0; D],
                        };
                        allocate_temp(
                            &mut buffers,
                            nid,
                            output,
                            &mut temp_count,
                            &mut peak_temp,
                            &mut cycle_peak,
                        );
                    }
                    NodeType::Direct => {
                        let input_data = resolve_input(node, &buffers, &input_map);
                        if output_node_set.contains(&nid) {
                            if let Some(pos) = graph.output_nodes.iter().position(|&out_id| out_id == nid) {
                                output_vectors[pos] = input_data;
                            }
                        } else {
                            allocate_temp(
                                &mut buffers,
                                nid,
                                input_data,
                                &mut temp_count,
                                &mut peak_temp,
                                &mut cycle_peak,
                            );
                        }
                    }
                    NodeType::Reduce => {
                        let mut acc = [0.0f32; D];
                        for &dep in &node.deps {
                            if let Some(data) = buffers.get(&dep) {
                                for i in 0..D {
                                    acc[i] += data[i];
                                }
                            }
                        }
                        if let Some(pos) = graph.output_nodes.iter().position(|&out_id| out_id == nid) {
                            output_vectors[pos] = acc;
                        }
                    }
                    NodeType::ZeroFill => {
                        if let Some(pos) = graph.output_nodes.iter().position(|&out_id| out_id == nid) {
                            output_vectors[pos] = [0.0; D];
                        }
                    }
                    NodeType::WriteBack => {
                        current_output_buffer = graph.output_nodes.len();
                        peak_output_buffer = peak_output_buffer.max(current_output_buffer);
                    }
                }

                retire_inputs(node, &mut buffers, &mut remaining_consumers, &mut temp_count);
                retire_input_block(node, &mut remaining_input_consumers, &mut current_input_buffer);
            }
        }

        let mut active_nodes: Vec<NodeId> = schedule
            .timed_nodes
            .iter()
            .filter(|timed| timed.start_cycle <= cycle && cycle < timed.finish_cycle)
            .map(|timed| timed.id)
            .collect();
        active_nodes.sort();
        per_cycle_trace.push(CycleSnapshot {
            cycle,
            active_nodes,
            input_buffer_occupancy: current_input_buffer,
            end_of_cycle_occupancy: temp_count,
            instant_peak_occupancy: cycle_peak,
            output_buffer_occupancy: current_output_buffer,
        });
    }

    SimResult {
        output_vectors,
        peak_temp_buffer: peak_temp,
        per_cycle_trace,
        peak_input_buffer_blocks: peak_input_buffer,
        output_buffer_blocks: current_output_buffer,
        peak_output_buffer_blocks: peak_output_buffer,
    }
}

fn is_input_addr(addr: usize) -> bool {
    addr < 10000
}

fn allocate_temp(
    buffers: &mut HashMap<NodeId, [f32; D]>,
    node_id: NodeId,
    data: [f32; D],
    temp_count: &mut usize,
    peak_temp: &mut usize,
    cycle_peak: &mut usize,
) {
    let existed = buffers.insert(node_id, data).is_some();
    if !existed {
        *temp_count += 1;
        *peak_temp = (*peak_temp).max(*temp_count);
        *cycle_peak = (*cycle_peak).max(*temp_count);
    }
}

fn retire_input_block(
    node: &crate::lower::SchedNode,
    remaining_input_consumers: &mut HashMap<usize, usize>,
    current_input_buffer: &mut usize,
) {
    if let Some(&addr) = node.input_slots.first() {
        if is_input_addr(addr) {
            if let Some(left) = remaining_input_consumers.get_mut(&addr) {
                *left = left.saturating_sub(1);
                if *left == 0 {
                    *current_input_buffer = current_input_buffer.saturating_sub(1);
                }
            }
        }
    }
}

fn retire_inputs(
    node: &crate::lower::SchedNode,
    buffers: &mut HashMap<NodeId, [f32; D]>,
    remaining_consumers: &mut HashMap<NodeId, usize>,
    temp_count: &mut usize,
) {
    for &dep in &node.deps {
        if let Some(left) = remaining_consumers.get_mut(&dep) {
            *left = left.saturating_sub(1);
            if *left == 0 && buffers.remove(&dep).is_some() {
                *temp_count = temp_count.saturating_sub(1);
            }
        }
    }
}

fn resolve_input(
    node: &crate::lower::SchedNode,
    buffers: &HashMap<NodeId, [f32; D]>,
    input_map: &HashMap<usize, [f32; D]>,
) -> [f32; D] {
    if let Some(&addr) = node.input_slots.first() {
        for &dep_id in &node.deps {
            if let Some(&data) = buffers.get(&dep_id) {
                return data;
            }
        }
        if let Some(&data) = input_map.get(&addr) {
            return data;
        }
    }
    [0.0; D]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{BlockMat, KernelRef, Mat2};
    use crate::lower::lower;
    use crate::schedule::{schedule_graph, SchedParams};

    #[test]
    fn test_single_contributor_output_row_is_written_back() {
        let m: Mat2 = [[2.0, 0.0], [0.0, 3.0]];
        let bm = BlockMat {
            rows: 1,
            cols: 1,
            grid: vec![vec![KernelRef::General(m)]],
        };
        let graph = lower(&bm).unwrap();
        let params = SchedParams::default_phase1();
        let schedule = schedule_graph(&graph, &params);
        let result = simulate(&graph, &schedule, &params, &[[4.0, 5.0]]);

        assert_eq!(result.output_vectors, vec![[8.0, 15.0]]);
    }

    #[test]
    fn test_temp_buffer_released_after_last_consumer() {
        let m: Mat2 = [[1.0, 0.0], [0.0, 1.0]];
        let bm = BlockMat {
            rows: 2,
            cols: 1,
            grid: vec![vec![KernelRef::General(m)], vec![KernelRef::General(m)]],
        };
        let graph = lower(&bm).unwrap();
        let params = SchedParams::default_phase1();
        let schedule = schedule_graph(&graph, &params);
        let result = simulate(&graph, &schedule, &params, &[[1.0, 2.0]]);

        assert!(result.peak_temp_buffer >= 1);
        assert_eq!(result.output_vectors, vec![[1.0, 2.0], [1.0, 2.0]]);
        assert_eq!(
            result
                .per_cycle_trace
                .iter()
                .map(|snapshot| snapshot.instant_peak_occupancy)
                .max(),
            Some(result.peak_temp_buffer)
        );
        assert_eq!(
            result
                .per_cycle_trace
                .last()
                .map(|snapshot| snapshot.end_of_cycle_occupancy),
            Some(0)
        );
        assert_eq!(result.per_cycle_trace.last().map(|snapshot| snapshot.input_buffer_occupancy), Some(0));
    }
}
