# bigsoup
Agent simulation backend

## Python frontend

Build the Rust extension (run once, and again after any Rust change):

```sh
maturin develop --release --skip-install
```

Run the app:

```sh
PYTHONPATH=python python3 python/econsim/runner.py
```
To toggle lines on graph, press `s`
To remove y autoscaling, press `y`


Benchmark speed:
```sh
cargo run --release --bin econsim
```

O'Brian 10 avril, 9h30 am, 5e etage pavillon C, CHUM