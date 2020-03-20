use crate::mem::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockState {
    Free,
    Usable,
    Unusable,
}

pub const BLOCK_SIZE: usize = 8 * 1024;
pub const BLOCK_BYTEMAP_MASK: isize = !(BLOCK_SIZE as isize - 1);

pub struct Block {
    pub memory: Address,
    pub start: Address,
    pub top: Address,
    pub end: Address,
    pub state: BlockState,
}

impl Block {
    pub fn new(memory: Address, size: usize) -> Self {
        Self {
            memory,
            start: memory,
            top: memory,
            end: memory.offset(size),
            state: BlockState::Free,
        }
    }

    pub fn size(&self) -> usize {
        self.end.offset_from(self.start)
    }

    pub fn allocate(&mut self, size: usize) -> Option<Address> {
        if self.top < self.end {
            if self.state == BlockState::Free {
                self.state = BlockState::Usable;
            }
            let mem = self.top;
            self.top = self.top.offset(size);
            Some(mem)
        } else {
            self.state = BlockState::Unusable;
            None
        }
    }
}
