use cgc::*;

use std::collections::HashMap;

fn main() {
    let s = gc_allocate(vec![gc_allocate(1)]);
    gc_add_root(s);
    gc_enable_stats();
    println!("{:?}",s);
    s.borrow_mut().pop();
    gc_collect_not_par();
    gc_collect_not_par();
    println!("{}",s.collected());
    gc_collect_not_par();
    println!("{}",s.collected());
}
