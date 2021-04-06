# Spade

A work in progress HDL that doesn't make you want to pull your hair out. Taking
inspiration from rust and clash, the goal is to make a safer more expressive
language than Verilog and VHDL but without sacrificing the ability for low
level control of the hardware.

```
entity counter(clk: clk, rst: bool, max: int<8>) -> int<8> {
    reg(clk) value reset (rst: 0) =
        if value == max {
            0
        }
        else {
            value + 1
        };
    value
}
```

## Getting started
1. Clone the repo `git clone git@gitlab.com:TheZoq2/spade.git`
2. Initalize the submodules `git submodules init && git submodules update`
3. Build the project `cargo build`
4. Build your spade code using `cargo run -- <input.spade> -o output.v`

## Planned features

- [x] Type inference
- [x] Strongly typed
- [ ] Structs
- [ ] Traits and generics
- [ ] Sum types and pattern matching
- [ ] Compile time checked pipelines
- [ ] ...
