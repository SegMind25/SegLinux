//! Edit gamescope's common options without discarding unrelated custom arguments.
pub fn option(args: &[String], flag: &str) -> u32 {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .and_then(|pair| pair[1].parse().ok())
        .unwrap_or(0)
}
pub fn set_option(args: &mut Vec<String>, flag: &str, value: u32) {
    let mut output = Vec::new();
    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        if arg == flag {
            if iter.peek().is_some_and(|next| !next.starts_with('-')) {
                iter.next();
            }
        } else {
            output.push(arg.clone());
        }
    }
    if value > 0 {
        output.extend([flag.to_string(), value.to_string()]);
    }
    *args = output;
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edits_frame_rate_without_discarding_custom_options() {
        let mut args = ["-f", "-r", "60", "--adaptive-sync", "-W", "1920"]
            .map(String::from)
            .to_vec();
        assert_eq!(option(&args, "-r"), 60);
        set_option(&mut args, "-r", 144);
        assert_eq!(args, ["-f", "--adaptive-sync", "-W", "1920", "-r", "144"]);
        set_option(&mut args, "-r", 0);
        assert_eq!(option(&args, "-r"), 0);
        assert!(args.contains(&"--adaptive-sync".into()));
    }
    #[test]
    fn missing_value_does_not_swallow_next_option() {
        let mut args = ["-r", "--adaptive-sync"].map(String::from).to_vec();
        set_option(&mut args, "-r", 60);
        assert_eq!(args, ["--adaptive-sync", "-r", "60"]);
    }
}
