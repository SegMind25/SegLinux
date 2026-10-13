//! The accounts shown on the sign-in screen: people's accounts from
//! `/etc/passwd`, not system ones.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    pub name: String,
    /// The full name of the account, or its user name when it has none.
    pub display_name: String,
}

/// Accounts with these user ids belong to people (useradd's defaults).
const PEOPLE: std::ops::Range<u32> = 1000..60000;

/// The people's accounts in `passwd` (the text of /etc/passwd), in order.
pub fn parse(passwd: &str) -> Vec<User> {
    passwd
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            let [name, _, uid, _, gecos, _, shell] = fields[..] else {
                return None;
            };
            let uid: u32 = uid.parse().ok()?;
            if !PEOPLE.contains(&uid)
                || name.is_empty()
                || shell.ends_with("/nologin")
                || shell.ends_with("/false")
            {
                return None;
            }
            let full = gecos.split(',').next().unwrap_or("").trim();
            Some(User {
                name: name.to_string(),
                display_name: if full.is_empty() { name } else { full }.to_string(),
            })
        })
        .collect()
}

pub fn list() -> Vec<User> {
    parse(&std::fs::read_to_string("/etc/passwd").unwrap_or_default())
}

/// The letter shown in a person's round picture.
pub fn initial(user: &User) -> String {
    user.display_name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_else(|| "?".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_people() {
        let passwd = "root:x:0:0::/root:/bin/bash\n\
                      greeter:x:964:964:greetd greeter user:/var/lib/greetd:/usr/bin/nologin\n\
                      alice:x:1000:1000:Alice Martin,,,:/home/alice:/bin/bash\n\
                      bob:x:1001:1001::/home/bob:/bin/zsh\n\
                      locked:x:1002:1002::/home/locked:/usr/bin/nologin\n\
                      nobody:x:65534:65534:Kernel Overflow User:/:/usr/bin/nologin\n\
                      broken line\n";
        let users = parse(passwd);
        assert_eq!(
            users,
            [
                User {
                    name: "alice".into(),
                    display_name: "Alice Martin".into()
                },
                User {
                    name: "bob".into(),
                    display_name: "bob".into()
                },
            ]
        );
        assert_eq!(initial(&users[0]), "A");
        assert_eq!(
            initial(&User {
                name: "x".into(),
                display_name: "élodie".into()
            }),
            "É"
        );
    }
}
