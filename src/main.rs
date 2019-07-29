use cgc::generational::*;

fn main() {
    let v = gc_allocate(vec![]);
    v.borrow_mut().push(vec![String::from("Hello,World!")]);
    gc_enable_stats();
    println!("HELLO");
    gc_collect_not_par();
}
