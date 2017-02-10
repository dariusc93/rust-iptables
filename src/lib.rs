// In the name of Allah

extern crate regex;
extern crate nix;

pub mod error;

use std::process::{Command, Output};
use regex::Regex;
use error::{IPTResult, IPTError};
use std::fs::File;
use std::os::unix::io::AsRawFd;
use nix::fcntl::{flock, FlockArg};
use std::vec::Vec;
use std::ffi::OsStr;

/// Contains the iptables command and shows if it supports -w and -C options.
/// Use `new` method to create a new instance of this struct.
pub struct IPTables {
    /// The utility command which must be 'iptables' or 'ip6tables'.
    pub cmd: &'static str,

    /// Indicates if iptables has -C (--check) option
    pub has_check: bool,

    /// Indicates if iptables has -w (--wait) option
    pub has_wait: bool,
}

/// Returns `None` because iptables only works on linux
#[cfg(not(target_os = "linux"))]
pub fn new(is_ipv6: bool) -> IPTResult<IPTables> {
    Err(IPTError {
        message: "iptables only works on Linux",
    })
}

/// Creates a new `IPTables` Result with the command of 'iptables' if `is_ipv6` is `false`, otherwise the command is 'ip6tables'.
#[cfg(target_os = "linux")]
pub fn new(is_ipv6: bool) -> IPTResult<IPTables> {
    let cmd = if is_ipv6 {
        "ip6tables"
    } else {
        "iptables"
    };

    let version_output = Command::new(cmd).arg("--version").output()?;
    let re = Regex::new(r"v(\d+)\.(\d+)\.(\d+)")?;
    let version_string = String::from_utf8_lossy(&version_output.stdout).into_owned();
    let versions = re.captures(&version_string).ok_or("invalid version number")?;
    let v_major = versions.get(1).ok_or("unable to get major version number")?.as_str().parse::<i32>()?;
    let v_minor = versions.get(2).ok_or("unable to get minor version number")?.as_str().parse::<i32>()?;
    let v_patch = versions.get(3).ok_or("unable to get patch version number")?.as_str().parse::<i32>()?;

    Ok(IPTables {
        cmd: cmd,
        has_check: (v_major > 1) || (v_major == 1 && v_minor > 4) || (v_major == 1 && v_minor == 4 && v_patch > 10),
        has_wait: (v_major > 1) || (v_major == 1 && v_minor > 4) || (v_major == 1 && v_minor == 4 && v_patch > 19),
    })
}

impl IPTables {
    /// Checks for the existence of the `rule` in the table/chain.
    /// Returns true if the rule exists.
    #[cfg(target_os = "linux")]
    pub fn exists(&self, table: &str, chain: &str, rule: &str) -> IPTResult<bool> {
        if !self.has_check {
            return self.exists_old_version(table, chain, rule);
        }

        match self.run(&[&["-t", table, "-C", chain], rule.split(" ").collect::<Vec<&str>>().as_slice()].concat()) {
            Ok(output) => Ok(output.status.success()),
            Err(err) => Err(err),
        }
    }

    /// Inserts `rule` in the `position` to the table/chain.
    /// Returns `true` if the rule is inserted.
    pub fn insert(&self, table: &str, chain: &str, rule: &str, position: i32) -> IPTResult<bool> {
        match self.run(&[&["-t", table, "-I", chain, &position.to_string()], rule.split(" ").collect::<Vec<&str>>().as_slice()].concat()) {
            Ok(output) => Ok(output.status.success()),
            Err(err) => Err(err),
        }
    }

    /// Inserts `rule` in the `position` to the table/chain if it does not exist.
    /// Returns `true` if the rule is inserted.
    pub fn insert_unique(&self, table: &str, chain: &str, rule: &str, position: i32) -> IPTResult<bool> {
        if self.exists(table, chain, rule)? {
            return Err(IPTError::Other("the rule exists in the table/chain"))
        }

        self.insert(table, chain, rule, position)
    }

    /// Replaces `rule` in the `position` to the table/chain.
    /// Returns `true` if the rule is replaced.
    pub fn replace(&self, table: &str, chain: &str, rule: &str, position: i32) -> IPTResult<bool> {
        match self.run(&[&["-t", table, "-R", chain, &position.to_string()], rule.split(" ").collect::<Vec<&str>>().as_slice()].concat()) {
            Ok(output) => Ok(output.status.success()),
            Err(err) => Err(err),
        }
    }

    /// Appends `rule` to the table/chain.
    /// Returns `true` if the rule is appended.
    pub fn append(&self, table: &str, chain: &str, rule: &str) -> IPTResult<bool> {
        match self.run(&[&["-t", table, "-A", chain], rule.split(" ").collect::<Vec<&str>>().as_slice()].concat()) {
            Ok(output) => Ok(output.status.success()),
            Err(err) => Err(err),
        }
    }

    /// Appends `rule` to the table/chain if it does not exist.
    /// Returns `true` if the rule is appended.
    pub fn append_unique(&self, table: &str, chain: &str, rule: &str) -> IPTResult<bool> {
        if self.exists(table, chain, rule)? {
            return Err(IPTError::Other("the rule exists in the table/chain"))
        }

        self.append(table, chain, rule)
    }

    /// Appends or replaces `rule` to the table/chain if it does not exist.
    /// Returns `true` if the rule is appended or replaced.
    pub fn append_replace(&self, table: &str, chain: &str, rule: &str) -> IPTResult<bool> {
        if self.exists(table, chain, rule)? {
            self.delete(table, chain, rule)?;
        }

        self.append(table, chain, rule)
    }

    /// Deletes `rule` from the table/chain.
    /// Returns `true` if the rule is deleted.
    pub fn delete(&self, table: &str, chain: &str, rule: &str) -> IPTResult<bool> {
        match self.run(&[&["-t", table, "-D", chain], rule.split(" ").collect::<Vec<&str>>().as_slice()].concat()) {
            Ok(output) => Ok(output.status.success()),
            Err(err) => Err(err),
        }
    }

    /// Deletes all repetition of the `rule` from the table/chain.
    /// Returns `true` if the rules are deleted.
    pub fn delete_all(&self, table: &str, chain: &str, rule: &str) -> IPTResult<bool> {
        while self.exists(table, chain, rule)? {
            self.delete(table, chain, rule)?;
        }
        Ok(true)
    }

    /// Lists rules in the table/chain.
    pub fn list(&self, table: &str, chain: &str) -> IPTResult<Vec<String>> {
        self.get_list(&["-t", table, "-S", chain])
    }

    /// Lists rules in the table.
    pub fn list_table(&self, table: &str) -> IPTResult<Vec<String>> {
        self.get_list(&["-t", table, "-S"])
    }

    /// Lists the name of each chain in the table.
    pub fn list_chains(&self, table: &str) -> IPTResult<Vec<String>> {
        let mut list = Vec::new();
        let output = String::from_utf8_lossy(&self.run(&["-t", table, "-S"])?.stdout).into_owned();
        for item in output.trim().split("\n") {
            let fields = item.split(" ").collect::<Vec<&str>>();
            if fields.len() > 1 && (fields[0] == "-P" || fields[0] == "-N") {
                list.push(fields[1].to_string());
            }
        }
        Ok(list)
    }

    /// Creates a new user-defined chain.
    /// Returns `true` if the chain is created.
    pub fn new_chain(&self, table: &str, chain: &str) -> IPTResult<bool> {
        match self.run(&["-t", table, "-N", chain]) {
            Ok(output) => Ok(output.status.success()),
            Err(err) => Err(err),
        }
    }

    /// Flushes (deletes all rules) a chain.
    /// Returns `true` if the chain is flushed.
    pub fn flush_chain(&self, table: &str, chain: &str) -> IPTResult<bool> {
        match self.run(&["-t", table, "-F", chain]) {
            Ok(output) => Ok(output.status.success()),
            Err(err) => Err(err),
        }
    }

    /// Renames a chain in the table.
    /// Returns `true` if the chain is renamed.
    pub fn rename_chain(&self, table: &str, old_chain: &str, new_chain: &str) -> IPTResult<bool> {
        match self.run(&["-t", table, "-E", old_chain, new_chain]) {
            Ok(output) => Ok(output.status.success()),
            Err(err) => Err(err),
        }
    }

    /// Deletes a user-defined chain in the table.
    /// Returns `true` if the chain is deleted.
    pub fn delete_chain(&self, table: &str, chain: &str) -> IPTResult<bool> {
        match self.run(&["-t", table, "-X", chain]) {
            Ok(output) => Ok(output.status.success()),
            Err(err) => Err(err),
        }
    }

    /// Flushes all chains in a table.
    /// Returns `true` if the chains are flushed.
    pub fn flush_table(&self, table: &str) -> IPTResult<bool> {
        match self.run(&["-t", table, "-F"]) {
            Ok(output) => Ok(output.status.success()),
            Err(err) => Err(err),
        }
    }

    fn exists_old_version(&self, table: &str, chain: &str, rule: &str) -> IPTResult<bool> {
        match self.run(&["-t", table, "-S"]) {
            Ok(output) => Ok(String::from_utf8_lossy(&output.stdout).into_owned().contains(&format!("-A {} {}", chain, rule))),
            Err(err) => Err(err),
        }
    }

    fn get_list<S: AsRef<OsStr>>(&self, args: &[S]) -> IPTResult<Vec<String>> {
        let mut list = Vec::new();
        let output = String::from_utf8_lossy(&self.run(args)?.stdout).into_owned();
        for item in output.trim().split("\n") {
            list.push(item.to_string())
        }
        Ok(list)
    }

    fn run<S: AsRef<OsStr>>(&self, args: &[S]) -> IPTResult<Output> {
        let mut file_lock = None;

        let mut output_cmd = Command::new(self.cmd);
        let output;

        if self.has_wait {
            output = output_cmd.args(args).arg("--wait").output()?;
        } else {
            file_lock = Some(File::create("/var/run/xtables_old.lock")?);

            let mut need_retry = true;
            while need_retry {
                match flock(file_lock.as_ref().unwrap().as_raw_fd(), FlockArg::LockExclusiveNonblock) {
                    Ok(_) => need_retry = false,
                    Err(e) => if e.errno() == nix::errno::EAGAIN {
                        // FIXME: may cause infinite loop
                        need_retry = true;
                    } else {
                        return Err(IPTError::Nix(e));
                    },
                }
            }
            output = output_cmd.args(args).output()?;
        }

        if !self.has_wait {
            match file_lock {
                Some(f) => drop(f),
                None => (),
            };
        }

        Ok(output)
    }
}
