//! Lowering 层：BlockMat → SchedGraph
//!
//! 对应规格附录 A 的调度图生成。
//! 使用 ExecKernelRef（优化后层类型）而不是原始 KernelRef。

use crate::ir::{BlockMat, KernelRef, Mat2, ValidationError};
use crate::optimize::is_scalar_kernel;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// 缓冲区地址
pub type BufferAddr = usize;

/// 参数存储地址
pub type ParamAddr = usize;

/// 节点标识
pub type NodeId = usize;

/// 优化后/执行层核引用——含 Scalar 变体
#[derive(Debug, Clone, PartialEq)]
pub enum ExecKernelRef {
    General(Mat2),
    Id,
    Zero,
    Scalar(f32),
}

#[derive(Debug, Clone, Eq)]
enum ParamKey {
    General([u32; 4]),
    Scalar(u32),
}

impl PartialEq for ParamKey {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::General(a), Self::General(b)) => a == b,
            (Self::Scalar(a), Self::Scalar(b)) => a == b,
            _ => false,
        }
    }
}

impl Hash for ParamKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::General(bits) => {
                0u8.hash(state);
                bits.hash(state);
            }
            Self::Scalar(bits) => {
                1u8.hash(state);
                bits.hash(state);
            }
        }
    }
}

impl ExecKernelRef {
    /// 从原始 KernelRef 转换，R3 识别标量核
    pub fn from_kernel_ref(k: &KernelRef) -> Self {
        match k {
            KernelRef::Zero => ExecKernelRef::Zero,
            KernelRef::Id => ExecKernelRef::Id,
            KernelRef::General(m) => {
                if let Some(c) = is_scalar_kernel(m) {
                    ExecKernelRef::Scalar(c)
                } else {
                    ExecKernelRef::General(*m)
                }
            }
        }
    }

    pub fn is_zero(&self) -> bool {
        matches!(self, ExecKernelRef::Zero)
    }
}

/// 调度图节点类型
#[derive(Debug, Clone, PartialEq)]
pub enum NodeType {
    InputRead,
    ParamLoad,
    Broadcast,
    Core(ExecKernelRef),
    Direct,
    Reduce,
    ZeroFill,
    WriteBack,
}

/// 调度图节点
#[derive(Debug, Clone)]
pub struct SchedNode {
    pub id: NodeId,
    pub node_type: NodeType,
    pub arity: usize,
    pub transfer_scalars: usize,
    pub input_slots: Vec<BufferAddr>,
    pub output_slots: Vec<BufferAddr>,
    pub param_handle: Option<ParamAddr>,
    pub deps: Vec<NodeId>,
}

/// 调度图
#[derive(Debug, Clone)]
pub struct SchedGraph {
    pub nodes: HashMap<NodeId, SchedNode>,
    pub output_nodes: Vec<NodeId>,
}

fn param_key(exec_ref: &ExecKernelRef) -> Option<ParamKey> {
    match exec_ref {
        ExecKernelRef::General(m) => Some(ParamKey::General([
            m[0][0].to_bits(),
            m[0][1].to_bits(),
            m[1][0].to_bits(),
            m[1][1].to_bits(),
        ])),
        ExecKernelRef::Scalar(c) => Some(ParamKey::Scalar(c.to_bits())),
        _ => None,
    }
}

/// BlockMat → SchedGraph
///
/// Lowering 顺序：
/// 1. 为需要广播的输入列建立 BroadcastNode
/// 2. 为每个非零块建立 CoreNode 或 DirectNode
/// 3. 按输出行建立 ReduceNode / ZeroFillNode
pub fn lower(bm: &BlockMat) -> Result<SchedGraph, ValidationError> {
    bm.validate()?;

    let mut next_id: NodeId = 0;
    let mut nodes = HashMap::new();

    // 每列的非零块行索引
    let non_zero_in_col: Vec<Vec<usize>> = (0..bm.cols)
        .map(|j| {
            (0..bm.rows)
                .filter(|&i| bm.grid[i][j] != KernelRef::Zero)
                .collect()
        })
        .collect();

    // 每行的非零块列索引
    let non_zero_in_row: Vec<Vec<usize>> = (0..bm.rows)
        .map(|i| {
            (0..bm.cols)
                .filter(|&j| bm.grid[i][j] != KernelRef::Zero)
                .collect()
        })
        .collect();

    let is_permutation = bm.rows == bm.cols
        && non_zero_in_row.iter().all(|cols| cols.len() == 1)
        && non_zero_in_col.iter().all(|rows| rows.len() == 1)
        && bm.grid.iter().all(|row| row.iter().all(|k| matches!(k, KernelRef::Id | KernelRef::Zero)));

    let live_input_cols: Vec<usize> = non_zero_in_col
        .iter()
        .enumerate()
        .filter_map(|(j, rows)| if rows.is_empty() { None } else { Some(j) })
        .collect();

    let mut unique_param_keys: HashMap<ParamKey, usize> = HashMap::new();
    for row in &bm.grid {
        for kernel in row {
            let exec_ref = ExecKernelRef::from_kernel_ref(kernel);
            if let Some(key) = param_key(&exec_ref) {
                unique_param_keys.entry(key).or_insert_with(|| match exec_ref {
                    ExecKernelRef::General(_) => Mat2::default().len() * Mat2::default()[0].len(),
                    ExecKernelRef::Scalar(_) => 1,
                    _ => 0,
                });
            }
        }
    }

    let input_read_id = if !is_permutation && !live_input_cols.is_empty() {
        let id = next_id;
        next_id += 1;
        nodes.insert(
            id,
            SchedNode {
                id,
                node_type: NodeType::InputRead,
                arity: live_input_cols.len(),
                transfer_scalars: live_input_cols.len() * crate::ir::D,
                input_slots: live_input_cols.iter().map(|&j| input_addr(j)).collect(),
                output_slots: live_input_cols.iter().map(|&j| input_addr(j)).collect(),
                param_handle: None,
                deps: vec![],
            },
        );
        Some(id)
    } else {
        None
    };

    let param_load_id = if !unique_param_keys.is_empty() {
        let id = next_id;
        next_id += 1;
        let transfer_scalars: usize = unique_param_keys.values().sum();
        nodes.insert(
            id,
            SchedNode {
                id,
                node_type: NodeType::ParamLoad,
                arity: unique_param_keys.len(),
                transfer_scalars,
                input_slots: vec![],
                output_slots: vec![],
                param_handle: None,
                deps: vec![],
            },
        );
        Some(id)
    } else {
        None
    };

    // ---- 1. BroadcastNode ----
    let mut broadcast_ids: HashMap<usize, NodeId> = HashMap::new();
    for j in 0..bm.cols {
        if non_zero_in_col[j].len() > 1 {
            let id = next_id;
            next_id += 1;
            broadcast_ids.insert(j, id);
            nodes.insert(
                id,
                SchedNode {
                    id,
                    node_type: NodeType::Broadcast,
                    arity: non_zero_in_col[j].len(),
                    transfer_scalars: crate::ir::D,
                    input_slots: vec![input_addr(j)],
                    output_slots: vec![temp_addr(id)],
                    param_handle: None,
                    deps: input_read_id.into_iter().collect(),
                },
            );
        }
    }

    // ---- 2. CoreNode / DirectNode ----
    let mut kernel_ids: HashMap<(usize, usize), NodeId> = HashMap::new();
    let mut param_counter: ParamAddr = 0;
    let mut shared_params: HashMap<ParamKey, ParamAddr> = HashMap::new();
    for i in 0..bm.rows {
        for j in 0..bm.cols {
            if bm.grid[i][j] == KernelRef::Zero {
                continue;
            }
            let id = next_id;
            next_id += 1;
            kernel_ids.insert((i, j), id);

            let exec_ref = ExecKernelRef::from_kernel_ref(&bm.grid[i][j]);
            let node_type = match &exec_ref {
                ExecKernelRef::Id => NodeType::Direct,
                _ => NodeType::Core(exec_ref.clone()),
            };

            let mut deps: Vec<NodeId> = broadcast_ids.get(&j).copied().into_iter().collect();
            if deps.is_empty() {
                if let Some(input_id) = input_read_id {
                    deps.push(input_id);
                }
            }

            let input_slot = if let Some(&bcast_id) = broadcast_ids.get(&j) {
                temp_addr(bcast_id)
            } else {
                input_addr(j)
            };

            let param_handle = if let Some(key) = param_key(&exec_ref) {
                if let Some(&handle) = shared_params.get(&key) {
                    Some(handle)
                } else {
                    let handle = param_counter;
                    param_counter += 1;
                    shared_params.insert(key, handle);
                    Some(handle)
                }
            } else {
                None
            };

            if param_handle.is_some() {
                if let Some(load_id) = param_load_id {
                    deps.push(load_id);
                }
            }

            nodes.insert(
                id,
                SchedNode {
                    id,
                    node_type,
                    arity: 1,
                    transfer_scalars: 0,
                    input_slots: vec![input_slot],
                    output_slots: vec![temp_addr(id)],
                    param_handle,
                    deps,
                },
            );
        }
    }

    // ---- 3. ReduceNode / DirectNode / ZeroFillNode ----
    let mut output_node_ids = Vec::with_capacity(bm.rows);
    for i in 0..bm.rows {
        let id = next_id;
        next_id += 1;

        let row_deps: Vec<NodeId> = non_zero_in_row[i]
            .iter()
            .filter_map(|&j| kernel_ids.get(&(i, j)).copied())
            .collect();

        let node_type = match row_deps.len() {
            0 => NodeType::ZeroFill,
            1 => NodeType::Direct,
            _ => NodeType::Reduce,
        };

        let input_slots: Vec<BufferAddr> = row_deps.iter().map(|&nid| temp_addr(nid)).collect();

        nodes.insert(
            id,
            SchedNode {
                id,
                node_type,
                arity: row_deps.len(),
                transfer_scalars: 0,
                input_slots,
                output_slots: vec![output_addr(i)],
                param_handle: None,
                deps: row_deps,
            },
        );
        output_node_ids.push(id);
    }

    if !is_permutation && !output_node_ids.is_empty() {
        let id = next_id;
        nodes.insert(
            id,
            SchedNode {
                id,
                node_type: NodeType::WriteBack,
                arity: output_node_ids.len(),
                transfer_scalars: bm.rows * crate::ir::D,
                input_slots: output_node_ids.iter().map(|&nid| temp_addr(nid)).collect(),
                output_slots: (0..bm.rows).map(output_addr).collect(),
                param_handle: None,
                deps: output_node_ids.clone(),
            },
        );
    }

    Ok(SchedGraph {
        nodes,
        output_nodes: output_node_ids,
    })
}

// 地址映射 helper
fn input_addr(block_idx: usize) -> BufferAddr {
    block_idx
}

fn output_addr(block_idx: usize) -> BufferAddr {
    10000 + block_idx
}

fn temp_addr(node_id: NodeId) -> BufferAddr {
    20000 + node_id
}

/// 资源类别
pub fn resource_class(nt: &NodeType) -> &'static str {
    match nt {
        NodeType::InputRead => "input",
        NodeType::ParamLoad => "load",
        NodeType::Broadcast => "bcast",
        NodeType::Reduce | NodeType::ZeroFill => "reduce",
        NodeType::WriteBack => "writeback",
        NodeType::Direct => "none",
        NodeType::Core(_) => "core",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::KernelRef;

    #[test]
    fn test_lower_2x3() {
        // 示例 1：2×3 全稠密
        let m1: Mat2 = [[1.0, 2.0], [3.0, 4.0]];
        let bm = BlockMat {
            rows: 2,
            cols: 3,
            grid: vec![
                vec![
                    KernelRef::General(m1),
                    KernelRef::General(m1),
                    KernelRef::General(m1),
                ],
                vec![
                    KernelRef::General(m1),
                    KernelRef::General(m1),
                    KernelRef::General(m1),
                ],
            ],
        };
        let graph = lower(&bm).unwrap();
        // 1 InputRead + 1 ParamLoad + 3 BroadcastNode + 6 CoreNode + 2 ReduceNode + 1 WriteBack
        assert_eq!(graph.nodes.len(), 14);
        assert_eq!(graph.output_nodes.len(), 2);
    }

    #[test]
    fn test_lower_reuses_param_handle_for_shared_params() {
        let m: Mat2 = [[1.0, 2.0], [3.0, 4.0]];
        let bm = BlockMat {
            rows: 1,
            cols: 2,
            grid: vec![vec![KernelRef::General(m), KernelRef::General(m)]],
        };
        let graph = lower(&bm).unwrap();
        let mut handles: Vec<ParamAddr> = graph
            .nodes
            .values()
            .filter_map(|node| node.param_handle)
            .collect();
        handles.sort_unstable();
        handles.dedup();
        assert_eq!(handles, vec![0]);
    }
}
