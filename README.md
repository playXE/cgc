# cgc 
cgc is a compacting garbage collector

# Advantages
- Fast.
- Usable in a real-time thread, because collection can occur in another thread. 
# Disadvantages
- You need define `Traceable` trait for every type that you want to GC.


