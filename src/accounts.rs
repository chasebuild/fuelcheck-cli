use crate::config::{TokenAccount, TokenAccounts};
use anyhow::{anyhow, Result};

#[derive(Debug, Clone, Default)]
pub struct AccountSelectionArgs {
    pub account: Option<String>,
    pub account_index: Option<usize>,
    pub all_accounts: bool,
}

#[derive(Debug, Clone)]
pub struct SelectedAccount {
    pub index: usize,
    pub account: TokenAccount,
}

pub fn select_accounts(
    token_accounts: Option<&TokenAccounts>,
    args: &AccountSelectionArgs,
) -> Result<Option<Vec<SelectedAccount>>> {
    if args.all_accounts && (args.account.is_some() || args.account_index.is_some()) {
        return Err(anyhow!(
            "--all-accounts cannot be combined with --account or --account-index"
        ));
    }
    if args.account.is_some() && args.account_index.is_some() {
        return Err(anyhow!("use --account or --account-index, not both"));
    }

    let Some(token_accounts) = token_accounts else {
        if args.account.is_some() || args.account_index.is_some() {
            return Err(anyhow!("no accounts configured"));
        }
        return Ok(None);
    };

    let accounts = token_accounts.accounts.clone().unwrap_or_default();
    if accounts.is_empty() {
        if args.account.is_some() || args.account_index.is_some() {
            return Err(anyhow!("no accounts configured"));
        }
        return Ok(None);
    }

    if args.all_accounts {
        let selected = accounts
            .into_iter()
            .enumerate()
            .map(|(index, account)| SelectedAccount { index, account })
            .collect();
        return Ok(Some(selected));
    }

    if let Some(index) = args.account_index {
        if index >= accounts.len() {
            return Err(anyhow!("account index {} out of range", index));
        }
        return Ok(Some(vec![SelectedAccount {
            index,
            account: accounts[index].clone(),
        }]));
    }

    if let Some(name) = args.account.as_deref() {
        let Some(index) = find_account_index(&accounts, name) else {
            return Err(anyhow!("account '{}' not found", name));
        };
        return Ok(Some(vec![SelectedAccount {
            index,
            account: accounts[index].clone(),
        }]));
    }

    let active = token_accounts.active_index.filter(|idx| *idx < accounts.len());
    let index = active.unwrap_or(0);
    Ok(Some(vec![SelectedAccount {
        index,
        account: accounts[index].clone(),
    }]))
}

pub fn account_label(account: &TokenAccount, index: usize) -> String {
    account
        .label
        .clone()
        .or_else(|| account.id.clone())
        .filter(|val| !val.trim().is_empty())
        .unwrap_or_else(|| format!("account-{}", index + 1))
}

pub fn find_account_index(accounts: &[TokenAccount], name: &str) -> Option<usize> {
    let needle = name.trim().to_lowercase();
    if needle.is_empty() {
        return None;
    }
    accounts.iter().position(|account| matches_account(account, &needle))
}

fn matches_account(account: &TokenAccount, needle: &str) -> bool {
    account
        .label
        .as_ref()
        .map(|val| val.trim().to_lowercase())
        .filter(|val| !val.is_empty())
        .map(|val| val == needle)
        .unwrap_or(false)
        || account
            .id
            .as_ref()
            .map(|val| val.trim().to_lowercase())
            .filter(|val| !val.is_empty())
            .map(|val| val == needle)
            .unwrap_or(false)
}
