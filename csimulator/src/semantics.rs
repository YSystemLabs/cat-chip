//! 语义层：eval、compose、结构构造函数
//!
//! 对应规格 §3.5 构造函数与 §4.4 求值语义。

use crate::ir::{BlockMat, KernelRef, Mat2, ValidationError, D};

// ============================================================
// 矩阵运算辅助
// ============================================================

/// d×d 零矩阵
pub fn zero_mat() -> Mat2 {
    [[0.0; D]; D]
}

/// d×d 单位矩阵
pub fn identity_mat() -> Mat2 {
    let mut m = zero_mat();
    for i in 0..D {
        m[i][i] = 1.0;
    }
    m
}

/// 矩阵加法
pub fn mat_add(a: &Mat2, b: &Mat2) -> Mat2 {
    let mut r = zero_mat();
    for i in 0..D {
        for j in 0..D {
            r[i][j] = a[i][j] + b[i][j];
        }
    }
    r
}

/// 矩阵乘法
pub fn mat_mul(a: &Mat2, b: &Mat2) -> Mat2 {
    let mut r = zero_mat();
    for i in 0..D {
        for j in 0..D {
            for k in 0..D {
                r[i][j] += a[i][k] * b[k][j];
            }
        }
    }
    r
}

/// 矩阵-向量乘
pub fn mat_vec_mul(m: &Mat2, v: &[f32; D]) -> [f32; D] {
    let mut r = [0.0f32; D];
    for i in 0..D {
        for j in 0..D {
            r[i] += m[i][j] * v[j];
        }
    }
    r
}

// ============================================================
// KernelRef 级别运算
// ============================================================

/// 核求值：KernelRef → Mat2
pub fn eval_kernel(k: &KernelRef) -> Mat2 {
    match k {
        KernelRef::General(m) => *m,
        KernelRef::Id => identity_mat(),
        KernelRef::Zero => zero_mat(),
    }
}

/// 核组合（矩阵乘法）
pub fn kernel_compose(g: &KernelRef, f: &KernelRef) -> KernelRef {
    match (g, f) {
        (KernelRef::Zero, _) | (_, KernelRef::Zero) => KernelRef::Zero,
        (KernelRef::Id, k) => k.clone(),
        (k, KernelRef::Id) => k.clone(),
        (KernelRef::General(m1), KernelRef::General(m2)) => {
            KernelRef::General(mat_mul(m1, m2))
        }
    }
}

/// 核求和（矩阵加法）
pub fn kernel_add(a: &KernelRef, b: &KernelRef) -> KernelRef {
    match (a, b) {
        (KernelRef::Zero, k) => k.clone(),
        (k, KernelRef::Zero) => k.clone(),
        (KernelRef::Id, KernelRef::Id) => {
            let mut m = zero_mat();
            for i in 0..D {
                m[i][i] = 2.0;
            }
            KernelRef::General(m)
        }
        (KernelRef::Id, KernelRef::General(m)) | (KernelRef::General(m), KernelRef::Id) => {
            KernelRef::General(mat_add(&identity_mat(), m))
        }
        (KernelRef::General(m1), KernelRef::General(m2)) => {
            KernelRef::General(mat_add(m1, m2))
        }
    }
}

/// 核数乘
pub fn kernel_scale(scalar: f32, k: &KernelRef) -> KernelRef {
    if scalar == 0.0 {
        return KernelRef::Zero;
    }
    match k {
        KernelRef::Zero => KernelRef::Zero,
        KernelRef::Id if scalar == 1.0 => KernelRef::Id,
        KernelRef::Id => {
            let mut m = zero_mat();
            for i in 0..D {
                m[i][i] = scalar;
            }
            KernelRef::General(m)
        }
        KernelRef::General(m) => {
            let mut scaled = zero_mat();
            for i in 0..D {
                for j in 0..D {
                    scaled[i][j] = scalar * m[i][j];
                }
            }
            KernelRef::General(scaled)
        }
    }
}

// ============================================================
// BlockMat 求值
// ============================================================

/// eval(BlockMat) → 展开为 (n*d) × (m*d) 密集矩阵（行主序）
pub fn eval(bm: &BlockMat) -> Vec<Vec<f32>> {
    let total_rows = bm.rows * D;
    let total_cols = bm.cols * D;
    let mut result = vec![vec![0.0f32; total_cols]; total_rows];
    for bi in 0..bm.rows {
        for bj in 0..bm.cols {
            let km = eval_kernel(&bm.grid[bi][bj]);
            for r in 0..D {
                for c in 0..D {
                    result[bi * D + r][bj * D + c] = km[r][c];
                }
            }
        }
    }
    result
}

// ============================================================
// Eager Flattening: compose
// ============================================================

/// compose(g, f) = g ∘ f —— eager flattening
pub fn compose(g: &BlockMat, f: &BlockMat) -> Result<BlockMat, ValidationError> {
    if g.cols != f.rows {
        return Err(ValidationError::DimensionMismatch {
            left_cols: g.cols,
            right_rows: f.rows,
        });
    }
    let n = g.cols; // 内部维度
    let p = g.rows;
    let m = f.cols;
    let mut grid = Vec::with_capacity(p);
    for i in 0..p {
        let mut row = Vec::with_capacity(m);
        for k in 0..m {
            let mut acc = KernelRef::Zero;
            for j in 0..n {
                let term = kernel_compose(&g.grid[i][j], &f.grid[j][k]);
                acc = kernel_add(&acc, &term);
            }
            row.push(acc);
        }
        grid.push(row);
    }
    Ok(BlockMat { rows: p, cols: m, grid })
}

/// 态射加法：对平行态射 f, g : A -> B，定义为 ∇_B ∘ (f ⊕ g) ∘ Δ_A。
pub fn morph_add(f: &BlockMat, g: &BlockMat) -> Result<BlockMat, ValidationError> {
    if f.rows != g.rows {
        return Err(ValidationError::DimensionMismatch {
            left_cols: f.rows,
            right_rows: g.rows,
        });
    }
    if f.cols != g.cols {
        return Err(ValidationError::DimensionMismatch {
            left_cols: f.cols,
            right_rows: g.cols,
        });
    }

    let delta = pair(&identity(f.cols)?, &identity(f.cols)?)?;
    let sigma = copair(&identity(f.rows)?, &identity(f.rows)?)?;
    let biproduct_map = bimap(&[f.clone(), g.clone()])?;
    let duplicated = compose(&biproduct_map, &delta)?;
    compose(&sigma, &duplicated)
}

/// 态射数乘：对 BlockMat 的每个核做逐块标量缩放。
pub fn morph_scale(scalar: f32, f: &BlockMat) -> BlockMat {
    let grid = f
        .grid
        .iter()
        .map(|row| row.iter().map(|kernel| kernel_scale(scalar, kernel)).collect())
        .collect();
    BlockMat {
        rows: f.rows,
        cols: f.cols,
        grid,
    }
}

// ============================================================
// 结构构造函数 (§3.5)
// ============================================================

/// kernelApp(M) → 1×1 BlockMat
pub fn kernel_app(m: Mat2) -> BlockMat {
    BlockMat {
        rows: 1,
        cols: 1,
        grid: vec![vec![KernelRef::General(m)]],
    }
}

/// proj(i, n) → 1×n BlockMat：第 i 列为 Id，其余为 Zero
pub fn proj(i: usize, n: usize) -> Result<BlockMat, ValidationError> {
    if n == 0 {
        return Err(ValidationError::ZeroDimension);
    }
    if i >= n {
        return Err(ValidationError::IndexOutOfRange {
            index: i,
            upper_bound: n,
        });
    }
    let row: Vec<KernelRef> = (0..n)
        .map(|j| if j == i { KernelRef::Id } else { KernelRef::Zero })
        .collect();
    Ok(BlockMat { rows: 1, cols: n, grid: vec![row] })
}

/// inj(i, n) → n×1 BlockMat：第 i 行为 Id，其余为 Zero
pub fn inj(i: usize, n: usize) -> Result<BlockMat, ValidationError> {
    if n == 0 {
        return Err(ValidationError::ZeroDimension);
    }
    if i >= n {
        return Err(ValidationError::IndexOutOfRange {
            index: i,
            upper_bound: n,
        });
    }
    let grid: Vec<Vec<KernelRef>> = (0..n)
        .map(|j| vec![if j == i { KernelRef::Id } else { KernelRef::Zero }])
        .collect();
    Ok(BlockMat { rows: n, cols: 1, grid })
}

/// copy(n) → n×1 BlockMat：每行均为 Id
pub fn copy(n: usize) -> Result<BlockMat, ValidationError> {
    if n < 2 {
        return Err(ValidationError::ArityTooSmall {
            op: "copy",
            min: 2,
            actual: n,
        });
    }
    Ok(BlockMat {
        rows: n,
        cols: 1,
        grid: vec![vec![KernelRef::Id]; n],
    })
}

/// merge(n) → 1×n BlockMat：每列均为 Id
pub fn merge(n: usize) -> Result<BlockMat, ValidationError> {
    if n < 2 {
        return Err(ValidationError::ArityTooSmall {
            op: "merge",
            min: 2,
            actual: n,
        });
    }
    Ok(BlockMat {
        rows: 1,
        cols: n,
        grid: vec![vec![KernelRef::Id; n]],
    })
}

/// identity(n) → n×n 块单位矩阵
pub fn identity(n: usize) -> Result<BlockMat, ValidationError> {
    if n == 0 {
        return Err(ValidationError::ZeroDimension);
    }
    let grid: Vec<Vec<KernelRef>> = (0..n)
        .map(|i| {
            (0..n)
                .map(|j| if i == j { KernelRef::Id } else { KernelRef::Zero })
                .collect()
        })
        .collect();
    Ok(BlockMat { rows: n, cols: n, grid })
}

/// pair(f, g) → 纵向拼接
pub fn pair(f: &BlockMat, g: &BlockMat) -> Result<BlockMat, ValidationError> {
    if f.cols != g.cols {
        return Err(ValidationError::DimensionMismatch {
            left_cols: f.cols,
            right_rows: g.cols,
        });
    }
    let mut grid = f.grid.clone();
    grid.extend(g.grid.iter().cloned());
    Ok(BlockMat {
        rows: f.rows + g.rows,
        cols: f.cols,
        grid,
    })
}

/// copair(f, g) → 横向拼接
pub fn copair(f: &BlockMat, g: &BlockMat) -> Result<BlockMat, ValidationError> {
    if f.rows != g.rows {
        return Err(ValidationError::DimensionMismatch {
            left_cols: f.rows,
            right_rows: g.rows,
        });
    }
    let grid: Vec<Vec<KernelRef>> = f
        .grid
        .iter()
        .zip(g.grid.iter())
        .map(|(rf, rg)| {
            let mut row = rf.clone();
            row.extend(rg.iter().cloned());
            row
        })
        .collect();
    Ok(BlockMat {
        rows: f.rows,
        cols: f.cols + g.cols,
        grid,
    })
}

/// bimap(fs) → 块对角排列
pub fn bimap(fs: &[BlockMat]) -> Result<BlockMat, ValidationError> {
    if fs.is_empty() {
        return Err(ValidationError::EmptyBimap);
    }
    let total_rows: usize = fs.iter().map(|f| f.rows).sum();
    let total_cols: usize = fs.iter().map(|f| f.cols).sum();
    let mut grid = vec![vec![KernelRef::Zero; total_cols]; total_rows];
    let mut row_offset = 0;
    let mut col_offset = 0;
    for f in fs {
        for i in 0..f.rows {
            for j in 0..f.cols {
                grid[row_offset + i][col_offset + j] = f.grid[i][j].clone();
            }
        }
        row_offset += f.rows;
        col_offset += f.cols;
    }
    Ok(BlockMat {
        rows: total_rows,
        cols: total_cols,
        grid,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::KernelRef;

    #[test]
    fn test_eval_identity() {
        let bm = identity(2).unwrap();
        let m = eval(&bm);
        // 应为 4×4 单位矩阵
        assert_eq!(m.len(), 4);
        for i in 0..4 {
            for j in 0..4 {
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!((m[i][j] - expected).abs() < 1e-7);
            }
        }
    }

    #[test]
    fn test_compose_identity() {
        let id2 = identity(2).unwrap();
        let result = compose(&id2, &id2).unwrap();
        assert_eq!(result.rows, 2);
        assert_eq!(result.cols, 2);
        for i in 0..2 {
            for j in 0..2 {
                if i == j {
                    assert_eq!(result.grid[i][j], KernelRef::Id);
                } else {
                    assert_eq!(result.grid[i][j], KernelRef::Zero);
                }
            }
        }
    }

    #[test]
    fn test_proj_inj_roundtrip() {
        // compose(proj(0,2), inj(0,2)) should be identity(1)
        let p = proj(0, 2).unwrap();
        let i = inj(0, 2).unwrap();
        let result = compose(&p, &i).unwrap();
        assert_eq!(result.rows, 1);
        assert_eq!(result.cols, 1);
        assert_eq!(result.grid[0][0], KernelRef::Id);
    }

    #[test]
    fn test_proj_inj_zero_when_indices_differ() {
        let p = proj(0, 2).unwrap();
        let i = inj(1, 2).unwrap();
        let result = compose(&p, &i).unwrap();
        assert_eq!(result.grid[0][0], KernelRef::Zero);
    }

    #[test]
    fn test_merge_copy_gives_scaled_identity() {
        let result = compose(&merge(3).unwrap(), &copy(3).unwrap()).unwrap();
        assert_eq!(result.rows, 1);
        assert_eq!(result.cols, 1);
        assert_eq!(result.grid[0][0], KernelRef::General([[3.0, 0.0], [0.0, 3.0]]));
    }

    #[test]
    fn test_pair_of_projections_is_identity() {
        let p0 = proj(0, 2).unwrap();
        let p1 = proj(1, 2).unwrap();
        let paired = pair(&p0, &p1).unwrap();
        assert_eq!(paired, identity(2).unwrap());
    }

    #[test]
    fn test_copair_of_injections_is_identity() {
        let i0 = inj(0, 2).unwrap();
        let i1 = inj(1, 2).unwrap();
        let copaired = copair(&i0, &i1).unwrap();
        assert_eq!(copaired, identity(2).unwrap());
    }

    #[test]
    fn test_bimap_distributes_over_compose() {
        let f1 = kernel_app([[1.0, 2.0], [0.0, 1.0]]);
        let f2 = kernel_app([[2.0, 0.0], [0.0, 2.0]]);
        let g1 = kernel_app([[0.0, 1.0], [1.0, 0.0]]);
        let g2 = kernel_app([[1.0, 0.0], [3.0, 1.0]]);

        let lhs = compose(&bimap(&[f1.clone(), f2.clone()]).unwrap(), &bimap(&[g1.clone(), g2.clone()]).unwrap()).unwrap();
        let rhs = bimap(&[
            compose(&f1, &g1).unwrap(),
            compose(&f2, &g2).unwrap(),
        ]).unwrap();

        assert_eq!(eval(&lhs), eval(&rhs));
    }

    #[test]
    fn test_morph_add_has_zero_unit() {
        let f = kernel_app([[1.0, 2.0], [3.0, 4.0]]);
        let zero = BlockMat {
            rows: 1,
            cols: 1,
            grid: vec![vec![KernelRef::Zero]],
        };

        assert_eq!(eval(&morph_add(&f, &zero).unwrap()), eval(&f));
        assert_eq!(eval(&morph_add(&zero, &f).unwrap()), eval(&f));
    }

    #[test]
    fn test_morph_add_is_associative() {
        let f = kernel_app([[1.0, 0.0], [0.0, 1.0]]);
        let g = kernel_app([[0.0, 1.0], [1.0, 0.0]]);
        let h = kernel_app([[2.0, 0.0], [0.0, 2.0]]);

        let lhs = morph_add(&morph_add(&f, &g).unwrap(), &h).unwrap();
        let rhs = morph_add(&f, &morph_add(&g, &h).unwrap()).unwrap();

        assert_eq!(eval(&lhs), eval(&rhs));
    }

    #[test]
    fn test_morph_add_is_commutative() {
        let f = kernel_app([[1.0, 2.0], [0.0, 1.0]]);
        let g = kernel_app([[0.0, 1.0], [1.0, 0.0]]);

        let lhs = morph_add(&f, &g).unwrap();
        let rhs = morph_add(&g, &f).unwrap();

        assert_eq!(eval(&lhs), eval(&rhs));
    }

    #[test]
    fn test_compose_distributes_over_morph_add() {
        let f = kernel_app([[1.0, 1.0], [0.0, 1.0]]);
        let g = kernel_app([[2.0, 0.0], [0.0, 2.0]]);
        let h = kernel_app([[0.0, 1.0], [1.0, 0.0]]);

        let left_lhs = compose(&h, &morph_add(&f, &g).unwrap()).unwrap();
        let left_rhs = morph_add(&compose(&h, &f).unwrap(), &compose(&h, &g).unwrap()).unwrap();
        assert_eq!(eval(&left_lhs), eval(&left_rhs));

        let right_lhs = compose(&morph_add(&f, &g).unwrap(), &h).unwrap();
        let right_rhs = morph_add(&compose(&f, &h).unwrap(), &compose(&g, &h).unwrap()).unwrap();
        assert_eq!(eval(&right_lhs), eval(&right_rhs));
    }

    #[test]
    fn test_biproduct_resolution_of_identity() {
        let term0 = compose(&inj(0, 2).unwrap(), &proj(0, 2).unwrap()).unwrap();
        let term1 = compose(&inj(1, 2).unwrap(), &proj(1, 2).unwrap()).unwrap();
        let lhs = morph_add(&term0, &term1).unwrap();
        let rhs = identity(2).unwrap();

        assert_eq!(eval(&lhs), eval(&rhs));
    }

    #[test]
    fn test_morph_add_rejects_non_parallel_morphisms() {
        let f = identity(2).unwrap();
        let g = proj(0, 2).unwrap();
        assert!(matches!(morph_add(&f, &g), Err(ValidationError::DimensionMismatch { .. })));
    }

    #[test]
    fn test_morph_scale_has_unit_and_zero() {
        let f = identity(2).unwrap();
        let zero = morph_scale(0.0, &f);

        assert_eq!(eval(&morph_scale(1.0, &f)), eval(&f));
        assert_eq!(
            eval(&zero),
            eval(&BlockMat {
                rows: 2,
                cols: 2,
                grid: vec![
                    vec![KernelRef::Zero, KernelRef::Zero],
                    vec![KernelRef::Zero, KernelRef::Zero],
                ],
            })
        );
    }

    #[test]
    fn test_morph_scale_action_is_associative() {
        let f = kernel_app([[1.0, 2.0], [3.0, 4.0]]);
        let lhs = morph_scale(2.0 * -0.5, &f);
        let rhs = morph_scale(2.0, &morph_scale(-0.5, &f));
        assert_eq!(eval(&lhs), eval(&rhs));
    }

    #[test]
    fn test_morph_scale_distributes_over_morph_add() {
        let f = kernel_app([[1.0, 0.0], [0.0, 1.0]]);
        let g = kernel_app([[0.0, 1.0], [1.0, 0.0]]);
        let lhs = morph_scale(3.0, &morph_add(&f, &g).unwrap());
        let rhs = morph_add(&morph_scale(3.0, &f), &morph_scale(3.0, &g)).unwrap();
        assert_eq!(eval(&lhs), eval(&rhs));
    }

    #[test]
    fn test_scalar_addition_distributes_on_morphism() {
        let f = kernel_app([[1.0, 2.0], [0.0, 1.0]]);
        let lhs = morph_scale(5.0, &f);
        let rhs = morph_add(&morph_scale(2.0, &f), &morph_scale(3.0, &f)).unwrap();
        assert_eq!(eval(&lhs), eval(&rhs));
    }

    #[test]
    fn test_constructor_validation() {
        assert!(matches!(proj(2, 2), Err(ValidationError::IndexOutOfRange { .. })));
        assert!(matches!(inj(0, 0), Err(ValidationError::ZeroDimension)));
        assert!(matches!(copy(1), Err(ValidationError::ArityTooSmall { .. })));
        assert!(matches!(merge(1), Err(ValidationError::ArityTooSmall { .. })));
        assert!(matches!(identity(0), Err(ValidationError::ZeroDimension)));
        assert!(matches!(bimap(&[]), Err(ValidationError::EmptyBimap)));
    }
}
