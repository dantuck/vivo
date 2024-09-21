# Vivo [![Latest Version]][crates.io] 
<!-- [![Conduct svg]][Code of Conduct] -->

[Latest Version]: https://img.shields.io/crates/v/vivo.svg
[crates.io]: https://crates.io/crates/vivo
<!-- [Conduct svg]: code-of-conduct.svg
[Code of Conduct]: CODE_OF_CONDUCT.md -->

<a href="https://www.buymeacoffee.com/dantuck" target="_blank"><img src="https://cdn.buymeacoffee.com/buttons/default-orange.png" alt="Buy Me A Coffee" style="height: 51px !important;width: 217px !important;" ></a>

## Install

### From Cargo 

`cargo install vivo`

### Build from Source

Alternatively, clone this repo and do the following:

- If Rust is not installed on your machine, follow the instructions on how to do that here: https://www.rust-lang.org/tools/install
- run `cargo build --release` to compile the binary
- go to `/target/release` and copy the `vivo` binary in your path: `/usr/bin`

## Usage

### Environment variable

## Example

```bash
$ ./vivo help
[WIP] Vivo - restic backup w/ sync to b2


Usage: vivo [OPTIONS] [task_name] [COMMAND]

Commands:
  test  does testing things
  help  Print this message or the help of the given subcommand(s)

Arguments:
  [task_name]  Optional task name to operate on

Options:
  -c, --config <FILE>  Sets a custom config file
  -d, --debug...       Turn debugging information on
      --dry-run        Perform a dry run without making any changes
  -h, --help           Print help
  -V, --version        Print version  
```

## License

<!-- Licensed under GNU General Public License, Version 3, 29 June 2007 ([LICENSE-GNU](LICENSE) or <https://www.gnu.org/licenses/gpl.html>) -->

### Contribution
