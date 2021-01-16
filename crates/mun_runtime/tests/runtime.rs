use mun_test::CompileAndRunTestDriver;
use std::io;

#[macro_use]
mod util;

#[test]
#[ignore]
fn multiple_modules() {
    let driver = CompileAndRunTestDriver::from_fixture(
        r#"
    //- /mun.toml
    [package]
    name="foo"
    version="0.0.0"

    //- /src/mod.mun
    pub fn main() -> i32 { foo::foo() }

    //- /src/foo.mun
    pub fn foo() -> i32 { 5 }
    "#,
        |builder| builder,
    )
    .expect("Failed to build test driver");

    assert_invoke_eq!(i32, 5, driver, "main");
}

#[test]
fn from_fixture() {
    let driver = CompileAndRunTestDriver::from_fixture(
        r#"
    //- /mun.toml
    [package]
    name="foo"
    version="0.0.0"

    //- /src/mod.mun
    pub fn main() -> i32 { 5 }
    "#,
        |builder| builder,
    )
    .expect("Failed to build test driver");
    assert_invoke_eq!(i32, 5, driver, "main");
}

#[test]
fn error_assembly_not_linkable() {
    let driver = CompileAndRunTestDriver::new(
        r"
    extern fn dependency() -> i32;
    
    pub fn main() -> i32 { dependency() }
    ",
        |builder| builder,
    );
    assert_eq!(
        format!("{}", driver.unwrap_err()),
        format!(
            "{}",
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("Failed to link: function `dependency` is missing."),
            )
        )
    );
}

#[test]
fn arg_missing_bug() {
    let driver = CompileAndRunTestDriver::new(
        r"
    pub fn fibonacci_n() -> i64 {
        let n = arg();
        fibonacci(n)
    }

    fn arg() -> i64 {
        5
    }

    fn fibonacci(n: i64) -> i64 {
        if n <= 1 {
            n
        } else {
            fibonacci(n - 1) + fibonacci(n - 2)
        }
    }",
        |builder| builder,
    );
    driver.unwrap();
}
