//! Oracle 层：端到端对照 harness
//!
//! 不重复实现 eval()，而是接收 BlockMat + 输入向量 + SimResult，
//! 对比功能语义路径与模拟器路径的输出差异。

use crate::ir::{BlockMat, D};
use crate::semantics::eval;
use crate::simulate::SimResult;

/// 对照结果
#[derive(Debug, Clone)]
pub struct OracleResult {
    /// 是否通过
    pub pass: bool,
    /// 最大绝对误差
    pub max_abs_error: f32,
    /// 最大相对误差
    pub max_rel_error: f32,
}

/// 端到端对照
///
/// `bm`: 优化后（或优化前）的 BlockMat
/// `input_blocks`: m 个 d 维输入块
/// `sim_result`: simulate() 的输出
/// `epsilon`: 相对误差阈值（规格建议 1e-6）
pub fn compare(
    bm: &BlockMat,
    input_blocks: &[[f32; D]],
    sim_result: &SimResult,
    epsilon: f32,
) -> OracleResult {
    // 功能路径：eval() → 密集矩阵 × 密集向量
    let dense_mat = eval(bm);
    let total_rows = bm.rows * D;
    let total_cols = bm.cols * D;

    // 展平输入向量
    let mut flat_input = vec![0.0f32; total_cols];
    for (j, block) in input_blocks.iter().enumerate() {
        for k in 0..D {
            flat_input[j * D + k] = block[k];
        }
    }

    // oracle 输出
    let mut oracle_output = vec![0.0f32; total_rows];
    for i in 0..total_rows {
        for j in 0..total_cols {
            oracle_output[i] += dense_mat[i][j] * flat_input[j];
        }
    }

    // 模拟器输出展平
    let mut sim_output = vec![0.0f32; total_rows];
    for (bi, block) in sim_result.output_vectors.iter().enumerate() {
        for k in 0..D {
            if bi * D + k < total_rows {
                sim_output[bi * D + k] = block[k];
            }
        }
    }

    // 对比
    let mut max_abs = 0.0f32;
    let mut max_rel = 0.0f32;
    for i in 0..total_rows {
        let abs_err = (oracle_output[i] - sim_output[i]).abs();
        let rel_err = if oracle_output[i].abs() > 1e-10 {
            abs_err / oracle_output[i].abs()
        } else {
            abs_err
        };
        max_abs = max_abs.max(abs_err);
        max_rel = max_rel.max(rel_err);
    }

    OracleResult {
        pass: max_rel < epsilon,
        max_abs_error: max_abs,
        max_rel_error: max_rel,
    }
}
