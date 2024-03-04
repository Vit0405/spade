use crate::{build_items, snapshot_error};

snapshot_error! {
    stage_outside_pipeline,
    "
    entity main(x: X) -> int<8> {
        reg;
    }
    "
}

snapshot_error! {
    register_count_is_required,
    "
    pipeline(3) main(x: X) -> int<8> {
        reg *;
    }
    "
}

snapshot_error! {
    wrong_enum_variant_items_opener,
    "
    enum foo {
        A(int: int<4>),
    }
    "
}

snapshot_error! {
    wrong_enum_variant_items_opener_but_very_wrong,
    "
    enum foo {
        B|bool|,
    }
    "
}

snapshot_error! {
    wrong_argument_list_points_to_correct_token,
    "
    entity foo(clk: clk, a: bool) -> bool {
        reg(clk) a = a;
        a
    }

    entity main(clk: clk) -> bool {
        inst foo{clk, true}
    }
    "
}

snapshot_error! {
    functions_do_not_allow_inst_entity,
    "
    entity Y() -> bool { false }

    fn X() -> bool {
        inst Y()
    }
    "
}

snapshot_error! {
    functions_do_not_allow_inst_pipeline,
    "
    pipeline(2) P() -> bool {
        reg;
            false
    }

    fn X() -> bool {
        inst(2) Y()
    }
    "
}

snapshot_error! {
    unit_value_fails_gracefully,
    "fn a() -> bool {
        ()
    }"
}

snapshot_error! {
    missing_pipeline_depth_parens_is_an_error,
    "pipeline a(clk: clock) -> bool {
        true
    }"
}

snapshot_error! {
    negative_tuple_index_error,
    "fn a() -> int<10> {
        x#-10
    }"
}

snapshot_error! {
    tuple_index_out_of_bounds_error,
    "fn a(b: int<2>, c: int<2> ) -> int<3> {
        let tup = (b, c);
        tup#2
    }"
}

snapshot_error! {
    unexpected_token,
    "fn a() -> int<8> {
        let x : = 10;
        x
    }"
}

snapshot_error! {
    no_reg_in_function,
    "fn a(clk: clock) -> int<8> {
        reg(clk) x = 10;
        x
    }"
}

snapshot_error! {
    lexer_error_unexpected_symbol,
    "fn a() {
        let x = ¤;
    }"
}

snapshot_error! {
    empty_decl_error,
    "fn a() {
        decl;
    }"
}

#[test]
fn three_generic_end_chars_work() {
    let code = r#"
        struct A<T> {}
        struct X {
            a: A<A<A<bool>>>
        }
        "#;

    build_items(code);
}

snapshot_error! {
    missing_argument_list_for_inst_method_works,
    "fn a() -> bool {
        a.inst b
    }"
}

snapshot_error! {
    missing_pipeline_depth_error,
    "
        entity a() -> bool {
            inst() x()
        }
    "
}

snapshot_error! {
    min_max_compiles,
    "
        entity a(b: int<8>, c: int<8>, d: int<8>) -> int<8> {
            std::ops::min(c, std::ops::max(b, d))
        }
    "
}

snapshot_error! {
    order_compiles,
    "
        entity a(b: int<8>, c: int<8>) -> (int<8>, int<8>) {
            std::ops::order(b, c)
        }
    "
}

snapshot_error! {
    non_statements_in_statement_comptime_is_error,
    "
        fn a() {
            $if a == 0{
                let _ = 0;
                false
            }
            true
        }
    "
}

snapshot_error! {
    good_eof_error_on_missing_dot_continuation,
    "fn a() -> bool { a."
}

snapshot_error! {
    good_eof_error_on_missing_function_body,
    "fn a() -> bool"
}

snapshot_error! {
    good_eof_error_on_missing_function_body_without_type_signature,
    "fn a() -> bool"
}

snapshot_error! {
    good_error_on_unexpected_body,
    "entity a() -> bool struct"
}

snapshot_error! {
    empty_file_is_valid,
    ""
}

snapshot_error!(
    missing_expression,
    "entity a() -> int<32> {
        if 0 else {4};
    }"
);

snapshot_error! {
    using_empty_identifier_a,
    "
    entity counter(clk: clock, rst: bool, max: int<8>) -> int<8> {
        reg(clk) value reset (rst: 0) =
            if value == max {
                0
            }
            else {
                conv::trunc(value + 1)
            };
        value
    }
    "
}

snapshot_error! {
    using_empty_identifier_b,
    "
    use conv;

    entity counter(clk: clock, rst: bool, max: int<8>) -> int<8> {
        reg(clk) value reset (rst: 0) =
            if value == max {
                0
            }
            else {
                conv::trunc(value + 1)
            };
        value
    }
    "
}

#[test]
fn square_wave_readme_example() {
    let code = r#"
    entity square_wave(clk: clock, rst: bool) -> bool {
       reg(clk) value reset (rst: false) = !value;
        value
    }
    "#;

    build_items(code);
}

#[test]
fn neq_operator_works() {
    let code = r#"
    fn neq(x: int<8>, y: int<8>) -> bool {
        x != y
    }
    "#;

    build_items(code);
}

snapshot_error! {
    tuple_index_points_to_the_right_thing,
    "fn test(a: (bool,)) -> bool {
        a#0#0
    }"
}

#[test]
fn inverted_port_type() {
    let code = r#"
    entity square_wave(clk: clock, x: ~&mut bool) -> bool __builtin__
    "#;

    build_items(code);
}

snapshot_error! {
    wal_traceable_with_unexpected_param_is_error,
    "
        #[wal_traceable(a, uses_clk, this_is_not_valid)]
        struct T {}
    "
}

snapshot_error! {
    wal_trace_does_not_accept_duplicate_clk,
    "
        #[wal_trace(clk=x, clk=x)]
        struct T {}
    "
}
snapshot_error! {
    wal_trace_does_not_accept_bad_parameter,
    "
        #[wal_trace(clk=x, not_a_param=x)]
        struct T {}
    "
}

snapshot_error! {
    required_parameter_message_is_helpful,
    "
        fn main() -> bool {
            #[wal_suffix()]
            let x = 0;
            x
        }
    "
}

snapshot_error! {
    multiple_resets_triggers_error,
    "
    entity main() -> bool {
        reg(clk) a reset(true: 0) reset(true: 0) = true;
        true
    }"
}

snapshot_error! {
    multiple_resets_triggers_error_with_initial,
    "
    entity main() -> bool {
        reg(clk) a reset(true: 0) initial(true) reset(true: 0) = true;
        true
    }"
}

snapshot_error! {
    multiple_initial_does_not_pass_compiler,
    "
    entity main() -> bool {
        reg(clk) a reset(true: 0) initial(true) initial(true) = true;
        true
    }"
}

#[test]
fn reset_and_initial_in_either_order_is_valid() {
    let code = r#"
    entity main(clk: clock) -> bool {
        reg(clk) a reset(true: false) initial(true) = true;
        reg(clk) a initial(true) reset(true: false) = true;
        true
    }"#;

    build_items(code);
}

#[test]
fn block_comment_is_ignored() {
    let code = "
        /* this is an error */
    ";

    build_items(code);
}

#[test]
fn nested_block_comment_is_ignored() {
    let code = "
        /* /* */ this is an error after a block comment */
    ";

    build_items(code);
}

snapshot_error! {
    unterminated_block_comment_is_error,
    "/*/**/"
}

snapshot_error! {
    missing_end_of_range_index_is_error,
    "
        fn top() -> bool {
            let a = [true, true, false];
            a[1:]
        }
    "
}

snapshot_error! {
    both_range_index_is_expr_and_missing_end_is_error,
    "
        fn top() -> bool {
            let a = [true, true, false];
            a[1+1:]
        }
    "
}
