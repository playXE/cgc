extern crate cgc;
use cgc::block::*;
use cgc::gc::*;
use cgc::mem::*;

fn main() {
    let mut map = std::collections::HashSet::new();
    let mem = commit(BLOCK_SIZE, false);
    map.insert(mem.to_usize());
    let mut b = Block::new(mem, BLOCK_SIZE);
    println!("{:?}", mem.to_ptr::<*const u8>());
    unsafe {
        let val = b
            .allocate(std::mem::size_of::<InnerGc<i32>>())
            .unwrap()
            .to_mut_ptr::<InnerGc<i32>>();
        let val = &*val;
        println!(
            "{:?}",
            map.contains(&(val as *const _ as *const u8 as usize))
        );
    }
}
