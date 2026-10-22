use regex::Regex;

pub(super) fn generate(full_name: &str, existing_names: &[String]) -> String {
    let first_name = full_name
        .split_whitespace()
        .next()
        .expect("validated signup name");
    let prefix = format!("{first_name}'s");
    let escaped = regex::escape(&prefix);
    let first = Regex::new(&format!(r"(?i)\A{escaped}\s+(?:Fizzy|Rizzy)\z"))
        .expect("valid first account pattern");
    let numbered = Regex::new(&format!(
        r"(?i)\A{escaped}\s+(1st|2nd|3rd|[0-9]+th)\s+(?:Fizzy|Rizzy)"
    ))
    .expect("valid numbered account pattern");
    let last_index = existing_names
        .iter()
        .filter_map(|name| {
            if first.is_match(name) {
                Some(1)
            } else {
                let ordinal = numbered.captures(name)?.get(1)?.as_str();
                if ordinal.eq_ignore_ascii_case("1st") {
                    Some(1)
                } else if ordinal.eq_ignore_ascii_case("2nd") {
                    Some(2)
                } else if ordinal.eq_ignore_ascii_case("3rd") {
                    Some(3)
                } else {
                    ordinal[..ordinal.len() - 2].parse::<u64>().ok()
                }
            }
        })
        .max()
        .unwrap_or(0);
    let next_index = last_index.saturating_add(1);
    if next_index == 1 {
        format!("{prefix} Rizzy")
    } else {
        let suffix = if (11..=13).contains(&(next_index % 100)) {
            "th"
        } else {
            match next_index % 10 {
                1 => "st",
                2 => "nd",
                3 => "rd",
                _ => "th",
            }
        };
        format!("{prefix} {next_index}{suffix} Rizzy")
    }
}

#[cfg(test)]
mod tests {
    use super::generate;

    #[test]
    fn continues_from_the_highest_matching_account() {
        let names = [
            "Newart's Rizzy".to_owned(),
            "Newart's 2nd Rizzy".to_owned(),
            "Newart's 12th Fizzy".to_owned(),
            "Other's 99th Rizzy".to_owned(),
        ];
        assert_eq!(generate("Newart Userbaum", &names), "Newart's 13th Rizzy");
        assert_eq!(generate("Other Userbaum", &names), "Other's 100th Rizzy");
    }
}
