use tempo_commonware_node::cli;

fn main() {
    if let Err(err) = cli::run() {
        eprintln!("node failed with error\n{err:?}");
    }
}
