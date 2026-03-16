//! 优化层：R1-R8 重写规则与结构统计
//!
//! 对应规格 §5.2。

use crate::ir::{BlockMat, KernelRef, Mat2, D};
use crate::semantics::identity_mat;

/// 结构统计摘要（§6.3 符号表）
#[derive(Debug, Clone, PartialEq)]
pub struct StructStats {
    /// 非零块数
    pub n_nz: usize,
    /// 需要真实核执行的块数（非 Id 非 Zero）
    pub n_gen: usize,
    /// 标量核数（R3 识别的 c·I_d 形式）
    pub n_scale: usize,
    /// 去重后的唯一参数矩阵数（R4 参数共享后）
    pub n_unique: usize,
    /// 去重后的唯一标量核参数数
    pub n_scale_u: usize,
    /// 每列非零块数 c_j（广播因子）
    pub col_counts: Vec<usize>,
    /// 每行非零块数 r_i（归约因子）
    pub row_counts: Vec<usize>,
    /// 死列数
    pub dead_cols: usize,
    /// 死行数
    pub dead_rows: usize,
    /// 单项行数
    pub single_rows: usize,
    /// 是否为置换矩阵
    pub is_permutation: bool,
}

/// 判断矩阵是否为标量核 c·I_d
pub fn is_scalar_kernel(m: &Mat2) -> Option<f32> {
    let c = m[0][0];
    for i in 0..D {
        for j in 0..D {
            let expected = if i == j { c } else { 0.0 };
            if (m[i][j] - expected).abs() > 1e-7 {
                return None;
            }
        }
    }
    Some(c)
}

/// 判断矩阵是否为单位矩阵
fn is_identity(m: &Mat2) -> bool {
    let id = identity_mat();
    for i in 0..D {
        for j in 0..D {
            if (m[i][j] - id[i][j]).abs() > 1e-7 {
                return false;
            }
        }
    }
    true
}

/// 判断矩阵是否为零矩阵
fn is_zero(m: &Mat2) -> bool {
    for i in 0..D {
        for j in 0..D {
            if m[i][j].abs() > 1e-7 {
                return false;
            }
        }
    }
    true
}

/// R1: 零块消去——将数值零矩阵标记为 ZeroKernel
/// R2: 恒等块特化——将数值单位矩阵标记为 IdKernel
/// R3: 标量核检测——识别 c·I_d 形式（标记保留在原 General 中，统计时识别）
pub fn optimize(bm: &BlockMat) -> BlockMat {
    let grid: Vec<Vec<KernelRef>> = bm
        .grid
        .iter()
        .map(|row| {
            row.iter()
                .map(|k| match k {
                    KernelRef::General(m) => {
                        if is_zero(m) {
                            // R1: 零块消去
                            KernelRef::Zero
                        } else if is_identity(m) {
                            // R2: 恒等块特化
                            KernelRef::Id
                        } else {
                            KernelRef::General(*m)
                        }
                    }
                    other => other.clone(),
                })
                .collect()
        })
        .collect();
    BlockMat {
        rows: bm.rows,
        cols: bm.cols,
        grid,
    }
}

/// 收集结构统计
pub fn collect_stats(bm: &BlockMat) -> StructStats {
    let mut n_nz = 0;
    let mut n_gen = 0;
    let mut n_scale = 0;
    let mut params: Vec<Mat2> = Vec::new();
    let mut scalar_params: Vec<u32> = Vec::new(); // f32 bits for dedup

    let mut col_counts = vec![0usize; bm.cols];
    let mut row_counts = vec![0usize; bm.rows];

    for i in 0..bm.rows {
        for j in 0..bm.cols {
            match &bm.grid[i][j] {
                KernelRef::Zero => {}
                KernelRef::Id => {
                    n_nz += 1;
                    col_counts[j] += 1;
                    row_counts[i] += 1;
                }
                KernelRef::General(m) => {
                    n_nz += 1;
                    n_gen += 1;
                    col_counts[j] += 1;
                    row_counts[i] += 1;

                    if let Some(c) = is_scalar_kernel(m) {
                        n_scale += 1;
                        let bits = c.to_bits();
                        if !scalar_params.contains(&bits) {
                            scalar_params.push(bits);
                        }
                    }

                    // 简单去重：逐元素比较（第一阶段 d=2，参数矩阵很小）
                    let is_dup = params.iter().any(|p| {
                        p.iter()
                            .flatten()
                            .zip(m.iter().flatten())
                            .all(|(a, b)| (a - b).abs() < 1e-7)
                    });
                    if !is_dup {
                        params.push(*m);
                    }
                }
            }
        }
    }

    let dead_cols = col_counts.iter().filter(|&&count| count == 0).count();
    let dead_rows = row_counts.iter().filter(|&&count| count == 0).count();
    let single_rows = row_counts.iter().filter(|&&count| count == 1).count();
    let is_permutation = bm.rows == bm.cols
        && row_counts.iter().all(|&count| count == 1)
        && col_counts.iter().all(|&count| count == 1)
        && bm.grid.iter().all(|row| row.iter().all(|k| matches!(k, KernelRef::Id | KernelRef::Zero)));

    StructStats {
        n_nz,
        n_gen,
        n_scale,
        n_unique: params.len(),
        n_scale_u: scalar_params.len(),
        col_counts,
        row_counts,
        dead_cols,
        dead_rows,
        single_rows,
        is_permutation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_scalar_kernel() {
        let m: Mat2 = [[3.0, 0.0], [0.0, 3.0]];
        assert_eq!(is_scalar_kernel(&m), Some(3.0));

        let m2: Mat2 = [[1.0, 0.5], [0.0, 1.0]];
        assert_eq!(is_scalar_kernel(&m2), None);
    }

    #[test]
    fn test_optimize_r1_r2() {
        let bm = BlockMat {
            rows: 1,
            cols: 2,
            grid: vec![vec![
                KernelRef::General([[1.0, 0.0], [0.0, 1.0]]),
                KernelRef::General([[0.0, 0.0], [0.0, 0.0]]),
            ]],
        };
        let opt = optimize(&bm);
        assert_eq!(opt.grid[0][0], KernelRef::Id);
        assert_eq!(opt.grid[0][1], KernelRef::Zero);
    }
}
