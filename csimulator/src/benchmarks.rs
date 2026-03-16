//! 代表性验证程序集合（V1-V8）
//!
//! 与技术规格说明 §8.4 对齐，供 CLI、测试和文档导出共用。

use crate::ir::{BlockMat, KernelRef, Mat2, D};
use crate::semantics::{compose, inj, kernel_app, morph_add, proj};

#[derive(Debug, Clone)]
pub struct BenchmarkCase {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub block_mat: BlockMat,
    pub input_blocks: Vec<[f32; D]>,
}

pub fn list_cases() -> Vec<BenchmarkCase> {
    vec![
        v1_dense(),
        v2_zero_block(),
        v3_identity(),
        v4_block_diagonal(),
        v5_permutation(),
        v6_large_mixed(),
        v7_biproduct_sum(),
        v8_hundred_dim_banded(),
    ]
}

pub fn get_case(case_id: &str) -> Option<BenchmarkCase> {
    list_cases()
        .into_iter()
        .find(|case| case.id.eq_ignore_ascii_case(case_id))
}

pub fn case_ids() -> Vec<&'static str> {
    list_cases().into_iter().map(|case| case.id).collect()
}

fn v1_dense() -> BenchmarkCase {
    let m1: Mat2 = [[1.0, 2.0], [3.0, 4.0]];
    let m2: Mat2 = [[0.5, -1.0], [1.0, 0.5]];
    let m3: Mat2 = [[2.0, 0.0], [0.0, 2.0]];
    BenchmarkCase {
        id: "V1",
        title: "全稠密基准",
        description: "2x3 全稠密 BlockMat，验证端到端正确性、广播与归约调度。",
        block_mat: BlockMat {
            rows: 2,
            cols: 3,
            grid: vec![
                vec![KernelRef::General(m1), KernelRef::General(m2), KernelRef::General(m3)],
                vec![KernelRef::General(m3), KernelRef::General(m1), KernelRef::General(m2)],
            ],
        },
        input_blocks: vec![[1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
    }
}

fn v2_zero_block() -> BenchmarkCase {
    let m: Mat2 = [[1.0, 2.0], [3.0, 4.0]];
    let zero: Mat2 = [[0.0, 0.0], [0.0, 0.0]];
    BenchmarkCase {
        id: "V2",
        title: "含零块",
        description: "2x3 含单个零块，验证 R1 零块消去。",
        block_mat: BlockMat {
            rows: 2,
            cols: 3,
            grid: vec![
                vec![KernelRef::General(m), KernelRef::General(m), KernelRef::General(zero)],
                vec![KernelRef::General(m), KernelRef::General(m), KernelRef::General(m)],
            ],
        },
        input_blocks: vec![[1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
    }
}

fn v3_identity() -> BenchmarkCase {
    let id: Mat2 = [[1.0, 0.0], [0.0, 1.0]];
    let m: Mat2 = [[1.0, 1.0], [0.0, 1.0]];
    BenchmarkCase {
        id: "V3",
        title: "含恒等块",
        description: "2x3 含显式恒等块，验证 R2 恒等块特化。",
        block_mat: BlockMat {
            rows: 2,
            cols: 3,
            grid: vec![
                vec![KernelRef::General(id), KernelRef::General(m), KernelRef::Zero],
                vec![KernelRef::Zero, KernelRef::General(m), KernelRef::General(m)],
            ],
        },
        input_blocks: vec![[2.0, 1.0], [1.0, 1.0], [0.5, 2.0]],
    }
}

fn v4_block_diagonal() -> BenchmarkCase {
    let a: Mat2 = [[2.0, 0.0], [0.0, 2.0]];
    let b: Mat2 = [[1.0, 0.0], [1.0, 1.0]];
    let c: Mat2 = [[0.0, 1.0], [1.0, 0.0]];
    BenchmarkCase {
        id: "V4",
        title: "块对角",
        description: "3x3 块对角矩阵，验证单项行特化与结构利用度。",
        block_mat: BlockMat {
            rows: 3,
            cols: 3,
            grid: vec![
                vec![KernelRef::General(a), KernelRef::Zero, KernelRef::Zero],
                vec![KernelRef::Zero, KernelRef::General(b), KernelRef::Zero],
                vec![KernelRef::Zero, KernelRef::Zero, KernelRef::General(c)],
            ],
        },
        input_blocks: vec![[1.0, 2.0], [2.0, 1.0], [3.0, 4.0]],
    }
}

fn v5_permutation() -> BenchmarkCase {
    BenchmarkCase {
        id: "V5",
        title: "置换矩阵",
        description: "3x3 置换矩阵，验证零执行成本的纯结构路径。",
        block_mat: BlockMat {
            rows: 3,
            cols: 3,
            grid: vec![
                vec![KernelRef::Zero, KernelRef::Id, KernelRef::Zero],
                vec![KernelRef::Zero, KernelRef::Zero, KernelRef::Id],
                vec![KernelRef::Id, KernelRef::Zero, KernelRef::Zero],
            ],
        },
        input_blocks: vec![[1.0, 2.0], [3.0, 4.0], [5.0, 6.0]],
    }
}

fn v6_large_mixed() -> BenchmarkCase {
    let a: Mat2 = [[1.0, 0.0], [0.0, 1.0]];
    let b: Mat2 = [[2.0, 0.0], [0.0, 2.0]];
    let c: Mat2 = [[1.0, 1.0], [0.0, 1.0]];
    BenchmarkCase {
        id: "V6",
        title: "大规模混合",
        description: "4x3 输入输出规模的混合结构矩阵，验证完整闭环与代价对比。",
        block_mat: BlockMat {
            rows: 3,
            cols: 4,
            grid: vec![
                vec![KernelRef::General(a), KernelRef::General(c), KernelRef::Zero, KernelRef::General(b)],
                vec![KernelRef::Zero, KernelRef::General(b), KernelRef::General(c), KernelRef::Zero],
                vec![KernelRef::General(b), KernelRef::Zero, KernelRef::General(a), KernelRef::General(c)],
            ],
        },
        input_blocks: vec![[1.0, 0.0], [0.0, 1.0], [2.0, 2.0], [1.0, 3.0]],
    }
}

fn v7_biproduct_sum() -> BenchmarkCase {
    let a: Mat2 = [[1.0, 2.0], [0.0, 1.0]];
    let b: Mat2 = [[2.0, 0.0], [1.0, 1.0]];
    let left = compose(
        &inj(0, 2).unwrap(),
        &compose(&kernel_app(a), &proj(0, 2).unwrap()).unwrap(),
    )
    .unwrap();
    let right = compose(
        &inj(1, 2).unwrap(),
        &compose(&kernel_app(b), &proj(1, 2).unwrap()).unwrap(),
    )
    .unwrap();

    BenchmarkCase {
        id: "V7",
        title: "双积分解加法",
        description: "用 morph_add 将两个局部态射注入到 B⊕B 上并求和，形成显式双积分解实验案例。",
        block_mat: morph_add(&left, &right).unwrap(),
        input_blocks: vec![[1.0, 1.0], [2.0, 3.0]],
    }
}

fn v8_hundred_dim_banded() -> BenchmarkCase {
    let block_count = 64usize;
    let scale_two: Mat2 = [[2.0, 0.0], [0.0, 2.0]];
    let scale_neg: Mat2 = [[-1.0, 0.0], [0.0, -1.0]];
    let scale_half: Mat2 = [[0.5, 0.0], [0.0, 0.5]];
    let shear: Mat2 = [[1.0, 1.0], [0.0, 1.0]];
    let swap: Mat2 = [[0.0, 1.0], [1.0, 0.0]];
    let mix: Mat2 = [[1.0, -1.0], [2.0, 0.0]];

    let mut grid = vec![vec![KernelRef::Zero; block_count]; block_count];

    // 区段 A：单项恒等行，覆盖 R2/R7。
    for row in 0..16 {
        grid[row][row] = KernelRef::Id;
    }

    // 区段 B：共享标量核，覆盖 R3/R4，并形成双项归约行。
    for row in 16..32 {
        let base = row - 16;
        grid[row][base] = KernelRef::General(scale_two);
        grid[row][16 + base] = KernelRef::General(scale_neg);
    }

    // 区段 C：共享一般核 + 部分标量核 + 部分 Id，制造中等 fanout 和混合结构。
    for row in 32..48 {
        let base = row - 32;
        grid[row][(2 * base) % 56] = KernelRef::General(shear);
        grid[row][(2 * base + 5) % 56] = KernelRef::General(swap);
        grid[row][(2 * base + 9) % 56] = if base % 2 == 0 {
            KernelRef::General(scale_half)
        } else {
            KernelRef::Id
        };
    }

    // 区段 D：热点列广播区，覆盖高 fanout + 四项归约。
    for row in 48..56 {
        grid[row][2] = KernelRef::General(shear);
        grid[row][18] = KernelRef::General(mix);
        grid[row][34] = KernelRef::General(scale_two);
        grid[row][50] = KernelRef::General(swap);
    }

    // 区段 E：单项一般核行，继续覆盖 R7，并保持尾部活列。
    for row in 56..60 {
        grid[row][row] = KernelRef::General(mix);
    }

    // 区段 F：60..63 保持全零，作为死行；列 60..63 也保持无消费者，作为死列。

    let input_blocks = (0..block_count)
        .map(|idx| {
            let x = (idx % 5) as f32 + 1.0;
            let y = ((idx * 3) % 7) as f32 - 2.0;
            [x, y]
        })
        .collect();

    BenchmarkCase {
        id: "V8",
        title: "128维多结构混合",
        description: "64 个 2D 基础块构成的 128 维输入输出混合 BlockMat，在单一案例中同时覆盖 Zero/Id/Scalar/共享参数/死行死列/单项行/高扇出广播与多项归约。",
        block_mat: BlockMat {
            rows: block_count,
            cols: block_count,
            grid,
        },
        input_blocks,
    }
}