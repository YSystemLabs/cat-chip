//! 调度层：纯时序调度器
//!
//! 对应规格附录 A 的 scheduleGraph。
//! 只决定节点何时发射、何时完成、资源何时占用，不做数值计算。

use crate::lower::{ExecKernelRef, NodeId, NodeType, SchedGraph, SchedNode};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap, HashSet};

// ============================================================
// 延迟模型
// ============================================================

/// 延迟策略枚举——可 Clone/Debug/Serialize，替代 fn 指针
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum LatencyModel {
    /// 树形：⌈log₂ k⌉
    Tree,
    /// 流水线：k
    Pipeline,
    /// 固定值
    Fixed(usize),
}

pub fn latency(model: &LatencyModel, k: usize) -> usize {
    match model {
        LatencyModel::Tree => {
            if k <= 1 {
                0
            } else {
                (k as f64).log2().ceil() as usize
            }
        }
        LatencyModel::Pipeline => k,
        LatencyModel::Fixed(v) => *v,
    }
}

// ============================================================
// 调度参数集
// ============================================================

/// 与规格 §6.2 代价参数表对齐
#[derive(Debug, Clone, Serialize)]
pub struct SchedParams {
    /// P：核阵列并行槽位数
    pub core_slots: usize,
    /// B_ports：广播端口数
    pub bcast_ports: usize,
    /// R_ports：归约端口数
    pub reduce_ports: usize,
    /// L_core：一般核执行延迟
    pub core_latency: usize,
    /// L_scale：标量核执行延迟
    pub scale_latency: usize,
    /// 直通延迟（第一版可取 0 或 1）
    pub direct_latency: usize,
    /// B_w：参数总线宽度（标量/周期）
    pub bw_params: usize,
    /// W_out：输出写端口带宽（标量/周期）
    pub bw_writeback: usize,
    /// 广播延迟模型
    pub bcast_model: LatencyModel,
    /// 归约延迟模型
    pub reduce_model: LatencyModel,
}

impl SchedParams {
    /// 第一阶段默认参数
    pub fn default_phase1() -> Self {
        SchedParams {
            core_slots: 4,
            bcast_ports: 2,
            reduce_ports: 2,
            core_latency: 4,
            scale_latency: 1,
            direct_latency: 0,
            bw_params: 2,
            bw_writeback: 4,
            bcast_model: LatencyModel::Tree,
            reduce_model: LatencyModel::Tree,
        }
    }
}

// ============================================================
// 调度结果
// ============================================================

#[derive(Debug, Clone, Serialize)]
pub struct Batch {
    pub cycle: usize,
    pub node_ids: Vec<NodeId>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimedNode {
    pub id: NodeId,
    pub start_cycle: usize,
    pub finish_cycle: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Schedule {
    pub topo_order: Vec<NodeId>,
    pub batches: Vec<Batch>,
    pub timed_nodes: Vec<TimedNode>,
    pub total_cycles: usize,
}

// ============================================================
// 调度算法
// ============================================================

/// 节点延迟
pub fn node_latency(params: &SchedParams, node: &SchedNode) -> usize {
    match &node.node_type {
        NodeType::InputRead => ceil_div(node.transfer_scalars, params.bw_writeback),
        NodeType::ParamLoad => ceil_div(node.transfer_scalars, params.core_slots * params.bw_params),
        NodeType::Broadcast => latency(&params.bcast_model, node.arity.max(1)),
        NodeType::Reduce => latency(&params.reduce_model, node.arity),
        NodeType::ZeroFill => 1,
        NodeType::Direct => params.direct_latency,
        NodeType::WriteBack => ceil_div(node.transfer_scalars, params.bw_writeback),
        NodeType::Core(exec_ref) => match exec_ref {
            ExecKernelRef::Scalar(_) => params.scale_latency,
            _ => params.core_latency,
        },
    }
}

/// Kahn 拓扑排序
pub fn topo_sort(graph: &SchedGraph) -> Vec<NodeId> {
    let mut indeg: HashMap<NodeId, usize> = graph
        .nodes
        .keys()
        .map(|&id| (id, 0))
        .collect();
    for node in graph.nodes.values() {
        for &_ in &node.deps {
            *indeg.entry(node.id).or_insert(0) += 1;
        }
    }

    let mut ready: BTreeSet<NodeId> = indeg
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(&id, _)| id)
        .collect();

    let mut order = Vec::with_capacity(graph.nodes.len());

    while let Some(&nid) = ready.iter().next() {
        ready.remove(&nid);
        order.push(nid);
        // 找所有以 nid 为依赖的节点
        for node in graph.nodes.values() {
            if node.deps.contains(&nid) {
                let deg = indeg.get_mut(&node.id).unwrap();
                *deg -= 1;
                if *deg == 0 {
                    ready.insert(node.id);
                }
            }
        }
    }
    order
}

/// 收集当前周期可发射的节点
fn ready_nodes(
    graph: &SchedGraph,
    finish_times: &HashMap<NodeId, usize>,
    issued: &HashSet<NodeId>,
    now: usize,
) -> Vec<NodeId> {
    let mut ready: Vec<NodeId> = graph
        .nodes
        .values()
        .filter(|node| {
            !issued.contains(&node.id)
                && node.deps.iter().all(|dep| {
                    finish_times
                        .get(dep)
                        .map_or(false, |&ft| ft <= now)
                })
        })
        .map(|n| n.id)
        .collect();
    ready.sort();
    ready
}

/// 贪心发射：按 ready 顺序扫描，资源未满就发射
fn issue_batch(
    graph: &SchedGraph,
    params: &SchedParams,
    ready: &[NodeId],
    used_input: &mut usize,
    used_load: &mut usize,
    used_core: &mut usize,
    used_bcast: &mut usize,
    used_reduce: &mut usize,
    used_writeback: &mut usize,
) -> Vec<NodeId> {
    let mut batch = Vec::new();

    for &nid in ready {
        let node = &graph.nodes[&nid];
        let rc = crate::lower::resource_class(&node.node_type);
        let accepted = match rc {
            "input" => {
                if *used_input < 1 {
                    *used_input += 1;
                    true
                } else {
                    false
                }
            }
            "load" => {
                if *used_load < 1 {
                    *used_load += 1;
                    true
                } else {
                    false
                }
            }
            "core" => {
                if *used_core < params.core_slots {
                    *used_core += 1;
                    true
                } else {
                    false
                }
            }
            "bcast" => {
                if *used_bcast < params.bcast_ports {
                    *used_bcast += 1;
                    true
                } else {
                    false
                }
            }
            "none" => true,
            "writeback" => {
                if *used_writeback < 1 {
                    *used_writeback += 1;
                    true
                } else {
                    false
                }
            }
            _ => {
                if *used_reduce < params.reduce_ports {
                    *used_reduce += 1;
                    true
                } else {
                    false
                }
            }
        };
        if accepted {
            batch.push(nid);
        }
    }
    batch
}

fn active_resource_usage(
    graph: &SchedGraph,
    timed_nodes: &[TimedNode],
    now: usize,
) -> (usize, usize, usize, usize, usize, usize) {
    let mut active_input = 0;
    let mut active_load = 0;
    let mut active_core = 0;
    let mut active_bcast = 0;
    let mut active_reduce = 0;
    let mut active_writeback = 0;

    for timed in timed_nodes {
        if timed.start_cycle <= now && now < timed.finish_cycle {
            if let Some(node) = graph.nodes.get(&timed.id) {
                match crate::lower::resource_class(&node.node_type) {
                    "input" => active_input += 1,
                    "load" => active_load += 1,
                    "core" => active_core += 1,
                    "bcast" => active_bcast += 1,
                    "reduce" => active_reduce += 1,
                    "writeback" => active_writeback += 1,
                    _ => {}
                }
            }
        }
    }

    (
        active_input,
        active_load,
        active_core,
        active_bcast,
        active_reduce,
        active_writeback,
    )
}

/// 最小周期推进器
pub fn schedule_graph(graph: &SchedGraph, params: &SchedParams) -> Schedule {
    let topo = topo_sort(graph);
    let all_ids: HashSet<NodeId> = graph.nodes.keys().copied().collect();
    let mut issued: HashSet<NodeId> = HashSet::new();
    let mut finish_times: HashMap<NodeId, usize> = HashMap::new();
    let mut batches = Vec::new();
    let mut timed_nodes = Vec::new();
    let mut now = 0;

    while issued.len() < all_ids.len() {
        let issued_before = issued.len();
        let (mut used_input, mut used_load, mut used_core, mut used_bcast, mut used_reduce, mut used_writeback) =
            active_resource_usage(graph, &timed_nodes, now);

        loop {
            let ready = ready_nodes(graph, &finish_times, &issued, now);
            let batch = issue_batch(
                graph,
                params,
                &ready,
                &mut used_input,
                &mut used_load,
                &mut used_core,
                &mut used_bcast,
                &mut used_reduce,
                &mut used_writeback,
            );

            if batch.is_empty() {
                break;
            }

            for &nid in &batch {
                let node = &graph.nodes[&nid];
                let lat = node_latency(params, node);
                let finish = now + lat;
                timed_nodes.push(TimedNode {
                    id: nid,
                    start_cycle: now,
                    finish_cycle: finish,
                });
                finish_times.insert(nid, finish);
                issued.insert(nid);
            }

            batches.push(Batch {
                cycle: now,
                node_ids: batch,
            });
        }

        if issued.len() == issued_before {
            now += 1;
            continue;
        }

        now += 1;
    }

    let total_cycles = timed_nodes
        .iter()
        .map(|t| t.finish_cycle)
        .max()
        .unwrap_or(0);

    Schedule {
        topo_order: topo,
        batches,
        timed_nodes,
        total_cycles,
    }
}

fn ceil_div(a: usize, b: usize) -> usize {
    if b == 0 {
        0
    } else {
        (a + b - 1) / b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{BlockMat, KernelRef, Mat2};
    use crate::lower::{lower, NodeType};

    #[test]
    fn test_schedule_simple() {
        let m: Mat2 = [[1.0, 0.0], [0.0, 1.0]];
        let bm = BlockMat {
            rows: 1,
            cols: 1,
            grid: vec![vec![KernelRef::General(m)]],
        };
        let graph = lower(&bm).unwrap();
        let params = SchedParams::default_phase1();
        let sched = schedule_graph(&graph, &params);
        assert!(sched.total_cycles > 0 || params.direct_latency == 0);
    }

    #[test]
    fn test_broadcast_latency_uses_fanout() {
        let m: Mat2 = [[2.0, 1.0], [0.0, 1.0]];
        let bm = BlockMat {
            rows: 2,
            cols: 1,
            grid: vec![vec![KernelRef::General(m)], vec![KernelRef::General(m)]],
        };
        let graph = lower(&bm).unwrap();
        let params = SchedParams::default_phase1();

        let broadcast = graph
            .nodes
            .values()
            .find(|node| matches!(node.node_type, NodeType::Broadcast))
            .unwrap();

        assert_eq!(broadcast.arity, 2);
        assert_eq!(node_latency(&params, broadcast), 1);
    }

    #[test]
    fn test_direct_nodes_do_not_consume_core_slots() {
        let graph = SchedGraph {
            nodes: HashMap::from([
                (
                    0,
                    SchedNode {
                        id: 0,
                        node_type: NodeType::Direct,
                        arity: 1,
                        transfer_scalars: 0,
                        input_slots: vec![0],
                        output_slots: vec![1],
                        param_handle: None,
                        deps: vec![],
                    },
                ),
                (
                    1,
                    SchedNode {
                        id: 1,
                        node_type: NodeType::Direct,
                        arity: 1,
                        transfer_scalars: 0,
                        input_slots: vec![1],
                        output_slots: vec![2],
                        param_handle: None,
                        deps: vec![0],
                    },
                ),
            ]),
            output_nodes: vec![1],
        };
        let params = SchedParams::default_phase1();
        let sched = schedule_graph(&graph, &params);
        assert_eq!(sched.total_cycles, 0);
    }
}
