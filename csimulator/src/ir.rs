//! IR 层：BlockMat 核心类型定义
//!
//! 对应规格 §3-§4，保持与 spec 一致的三变体 KernelRef。

/// 块宽常量（第一阶段 d=2）
pub const D: usize = 2;

/// 2×2 矩阵，行主序
pub type Mat2 = [[f32; D]; D];

/// 核引用——原始 IR 层，仅三变体
#[derive(Debug, Clone, PartialEq)]
pub enum KernelRef {
    /// 一般 d×d 参数矩阵
    General(Mat2),
    /// 恒等核 I_d
    Id,
    /// 零核 0_{d×d}
    Zero,
}

/// 块矩阵 IR 节点（唯一 IR）
///
/// `grid[i][j]` 为第 i 行第 j 列的 KernelRef，
/// 表示 n×m 块矩阵 BlockMat(rows, cols, grid)。
#[derive(Debug, Clone, PartialEq)]
pub struct BlockMat {
    /// 输出块数 n
    pub rows: usize,
    /// 输入块数 m
    pub cols: usize,
    /// n × m 网格
    pub grid: Vec<Vec<KernelRef>>,
}

/// 类型签名：(源类型, 目标类型) = (Blocks(m), Blocks(n))
impl BlockMat {
    pub fn type_of(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }

    /// 网格维度校验
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.rows == 0 || self.cols == 0 {
            return Err(ValidationError::ZeroDimension);
        }
        if self.grid.len() != self.rows {
            return Err(ValidationError::RowCountMismatch {
                expected: self.rows,
                actual: self.grid.len(),
            });
        }
        for (i, row) in self.grid.iter().enumerate() {
            if row.len() != self.cols {
                return Err(ValidationError::ColCountMismatch {
                    row: i,
                    expected: self.cols,
                    actual: row.len(),
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ValidationError {
    ZeroDimension,
    RowCountMismatch { expected: usize, actual: usize },
    ColCountMismatch { row: usize, expected: usize, actual: usize },
    DimensionMismatch { left_cols: usize, right_rows: usize },
    IndexOutOfRange { index: usize, upper_bound: usize },
    ArityTooSmall { op: &'static str, min: usize, actual: usize },
    EmptyBimap,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroDimension => write!(f, "BlockMat dimension must be >= 1"),
            Self::RowCountMismatch { expected, actual } => {
                write!(f, "expected {expected} rows, got {actual}")
            }
            Self::ColCountMismatch { row, expected, actual } => {
                write!(f, "row {row}: expected {expected} cols, got {actual}")
            }
            Self::DimensionMismatch { left_cols, right_rows } => {
                write!(f, "compose dimension mismatch: left.cols={left_cols} != right.rows={right_rows}")
            }
            Self::IndexOutOfRange { index, upper_bound } => {
                write!(f, "index {index} out of range, expected < {upper_bound}")
            }
            Self::ArityTooSmall { op, min, actual } => {
                write!(f, "{op} expects arity >= {min}, got {actual}")
            }
            Self::EmptyBimap => write!(f, "bimap expects at least one morph"),
        }
    }
}

impl std::error::Error for ValidationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_ok() {
        let bm = BlockMat {
            rows: 2,
            cols: 3,
            grid: vec![
                vec![KernelRef::Id, KernelRef::Zero, KernelRef::Zero],
                vec![KernelRef::Zero, KernelRef::Id, KernelRef::Zero],
            ],
        };
        assert!(bm.validate().is_ok());
    }

    #[test]
    fn test_validate_zero_dim() {
        let bm = BlockMat { rows: 0, cols: 1, grid: vec![] };
        assert!(matches!(bm.validate(), Err(ValidationError::ZeroDimension)));
    }
}
