#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSize {
    pub columns: usize,
    pub rows: usize,
}

impl GridSize {
    pub fn new(columns: usize, rows: usize) -> Self {
        Self { columns, rows }
    }
}
