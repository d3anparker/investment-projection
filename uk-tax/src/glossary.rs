//! What the UK's own words mean, as [`taxkit::GlossaryEntry`] data.
//!
//! **Data, like [`crate::tables`] -- not mechanism.** These are the terms this
//! crate puts on screen: its account labels, the allowances its figures are net
//! of, and the rules a reader has to know to make sense of a number. A
//! consumer renders them and learns nothing about the UK by doing so.
//!
//! Two disciplines, both inherited from `AccountKind::note`:
//!
//! * **Descriptive, never advisory.** Say what a rule does and which figure it
//!   moves. Never what a reader should do about it.
//! * **No rates, no thresholds.** Figures live in [`crate::tables`] and change
//!   yearly; a definition that quotes one is a second place to update and a
//!   silent way to go stale. Name the allowance, do not price it.
//!
//! Every account kind in [`crate::tables::UK_ACCOUNTS`] has an entry whose `id`
//! is that kind's id, which is what the coverage test checks -- so a new
//! account kind cannot ship unexplained.

use taxkit::GlossaryEntry;

use crate::tables::ids;

/// Topic labels, so the grouping reads the same in every entry.
mod topics {
    pub const ACCOUNTS: &str = "Accounts";
    pub const INCOME: &str = "Income tax";
    pub const GAINS: &str = "Capital gains";
    pub const RETIREMENT: &str = "Taking a pension";
}

/// The UK glossary, in presentation order and contiguous by topic.
pub const UK_GLOSSARY: &[GlossaryEntry] = &[
    // --- accounts -----------------------------------------------------------
    GlossaryEntry {
        id: ids::STOCKS_ISA,
        term: "Stocks & Shares ISA",
        also: "S&S ISA",
        definition: "A tax-free wrapper for investments. Growth and withdrawals \
                     are free of both income tax and capital gains tax, so this \
                     projection charges nothing when money comes out of one.",
        seen_in: "The account column, and the zero in the tax column beside it.",
        topic: topics::ACCOUNTS,
        see_also: &["capital_gains_tax"],
    },
    GlossaryEntry {
        id: ids::CASH_ISA,
        term: "Cash ISA",
        also: "",
        definition: "The same tax-free wrapper, holding cash rather than \
                     investments. Taxed identically here; the difference is the \
                     return you would enter, not the tax.",
        seen_in: "The account column.",
        topic: topics::ACCOUNTS,
        see_also: &[ids::STOCKS_ISA],
    },
    GlossaryEntry {
        id: ids::LIFETIME_ISA,
        term: "Lifetime ISA",
        also: "LISA",
        definition: "A tax-free wrapper with a government bonus on \
                     contributions and a withdrawal charge outside its intended \
                     uses. The bonus and the charge are not modelled -- enter \
                     what you expect to hold, and read withdrawals as \
                     qualifying ones.",
        seen_in: "The account column.",
        topic: topics::ACCOUNTS,
        see_also: &[ids::STOCKS_ISA],
    },
    GlossaryEntry {
        id: ids::JUNIOR_ISA,
        term: "Junior ISA",
        also: "JISA",
        definition: "A tax-free wrapper held for a child, which becomes theirs \
                     at 18. Taxed as an ISA here; the age at which it can be \
                     reached is not modelled.",
        seen_in: "The account column.",
        topic: topics::ACCOUNTS,
        see_also: &[ids::STOCKS_ISA],
    },
    GlossaryEntry {
        id: ids::PREMIUM_BONDS,
        term: "Premium Bonds",
        also: "",
        definition: "Prizes rather than interest, and free of tax. Modelled as \
                     an untaxed holding, so the return you enter stands for an \
                     average prize rate rather than a guaranteed one.",
        seen_in: "The account column.",
        topic: topics::ACCOUNTS,
        see_also: &[],
    },
    GlossaryEntry {
        id: ids::GIA,
        term: "General investment account",
        also: "GIA, dealing account, unwrapped",
        definition: "An ordinary investment account with no tax wrapper. Gains \
                     are charged to capital gains tax when you sell, which is \
                     why this kind asks what the holding cost you.",
        seen_in: "The cost box beside the account picker, and the tax column.",
        topic: topics::ACCOUNTS,
        see_also: &["capital_gains_tax", "cost_basis"],
    },
    GlossaryEntry {
        id: ids::VCT_EIS,
        term: "VCT or EIS holding",
        also: "Venture Capital Trust, Enterprise Investment Scheme",
        definition: "Higher-risk holdings carrying income tax relief on the way \
                     in and, once qualifying conditions are met, tax-free \
                     growth. The reliefs and their conditions are not modelled; \
                     the holding is projected as untaxed on the way out.",
        seen_in: "The account column.",
        topic: topics::ACCOUNTS,
        see_also: &[],
    },
    GlossaryEntry {
        id: ids::SIPP,
        term: "SIPP",
        also: "Self-invested personal pension",
        definition: "A personal pension you direct yourself. Part of what you \
                     take can be tax-free cash and the rest is taxed as income, \
                     and it cannot normally be reached before the minimum \
                     access age.",
        seen_in: "The tax column, and the message if a projection draws on one \
                  too early.",
        topic: topics::ACCOUNTS,
        see_also: &["tax_free_cash", "minimum_access_age", "marginal_rate"],
    },
    GlossaryEntry {
        id: ids::WORKPLACE_DC,
        term: "Workplace pension (defined contribution)",
        also: "Workplace DC, occupational money purchase",
        definition: "A pension pot built through an employer. Taxed on the way \
                     out exactly as a SIPP is, so the two behave identically in \
                     this projection.",
        seen_in: "The tax column.",
        topic: topics::ACCOUNTS,
        see_also: &[ids::SIPP, "tax_free_cash"],
    },
    GlossaryEntry {
        id: ids::DEFINED_BENEFIT,
        term: "Defined benefit pension",
        also: "DB, final salary, career average",
        definition: "A promised income rather than a pot with a value, so there \
                     is nothing here to draw down. If you have one, its income \
                     belongs in the other taxable income box, where it pushes \
                     the rest of your withdrawals up the rate bands.",
        seen_in: "The other taxable income box, not the holdings list.",
        topic: topics::ACCOUNTS,
        see_also: &["other_income", "marginal_rate"],
    },
    GlossaryEntry {
        id: ids::ONSHORE_BOND,
        term: "Onshore investment bond",
        also: "",
        definition: "An insurance wrapper taxed under its own chargeable-event \
                     rules, with top-slicing relief and a credit for tax paid \
                     within the fund. Those rules are not modelled, so the \
                     holding is projected untaxed and its figures should be read \
                     as indicative only.",
        seen_in: "The account column, and the note that says it is not modelled.",
        topic: topics::ACCOUNTS,
        see_also: &[],
    },
    GlossaryEntry {
        id: ids::OFFSHORE_BOND,
        term: "Offshore investment bond",
        also: "",
        definition: "The same wrapper written outside the UK, so gains roll up \
                     without tax inside the fund and the whole gain is charged \
                     as income when it comes out. Also not modelled here.",
        seen_in: "The account column, and the note that says it is not modelled.",
        topic: topics::ACCOUNTS,
        see_also: &[ids::ONSHORE_BOND],
    },
    // --- income tax ---------------------------------------------------------
    GlossaryEntry {
        id: "personal_allowance",
        term: "Personal allowance",
        also: "",
        definition: "The slice of income charged at nothing before any rate \
                     applies. It resets each tax year, so spreading withdrawals \
                     across years uses more of it than taking them in one.",
        seen_in: "The unclaimed-allowance column in the drawdown comparison.",
        topic: topics::INCOME,
        see_also: &["allowance_taper", "marginal_rate", "unused_allowance"],
    },
    GlossaryEntry {
        id: "allowance_taper",
        term: "Allowance withdrawal",
        also: "The 60% trap, tapered personal allowance",
        definition: "Above a threshold the personal allowance is taken away as \
                     income rises, so each extra pound is charged at its own \
                     rate and again through the allowance it removes. The true \
                     marginal rate in that stretch is well above the headline \
                     one, and this projection charges it as such.",
        seen_in: "The tax column, when a drawdown reaches that stretch.",
        topic: topics::INCOME,
        see_also: &["personal_allowance", "marginal_rate"],
    },
    GlossaryEntry {
        id: "marginal_rate",
        term: "Marginal rate",
        also: "",
        definition: "The rate charged on the next pound you take, as opposed to \
                     the average across everything you have taken. It is what \
                     the drawdown strategies compare, because it is what \
                     changes when money moves between accounts.",
        seen_in: "The strategy comparison, and the tax column month by month.",
        topic: topics::INCOME,
        see_also: &["personal_allowance", "tax_jurisdiction"],
    },
    GlossaryEntry {
        id: "tax_jurisdiction",
        term: "Tax jurisdiction",
        also: "England, Wales, Scotland, Northern Ireland",
        definition: "Income tax rates and bands on earned and pension income \
                     are set separately in Scotland, so which of the four \
                     applies changes what a pension withdrawal costs. Capital \
                     gains and the allowances are UK-wide.",
        seen_in: "The region picker beside the tax settings.",
        topic: topics::INCOME,
        see_also: &["marginal_rate", "capital_gains_tax"],
    },
    GlossaryEntry {
        id: "other_income",
        term: "Other taxable income",
        also: "",
        definition: "Income you already expect to receive -- a defined benefit \
                     pension, the state pension, rent, work. It is not part of \
                     the pot, but it fills the lower rate bands first, so \
                     withdrawals on top of it are charged at a higher rate.",
        seen_in: "The tax settings, and every taxed withdrawal figure.",
        topic: topics::INCOME,
        see_also: &["marginal_rate", ids::DEFINED_BENEFIT],
    },
    GlossaryEntry {
        id: "frozen_thresholds",
        term: "Frozen thresholds",
        also: "Fiscal drag",
        definition: "Allowances and band edges that stay at the same cash \
                     figure while incomes rise, which quietly increases tax over \
                     a long projection. They are held flat here unless you ask \
                     for yearly uprating, because that is what is currently \
                     legislated.",
        seen_in: "The uprating control in the tax settings.",
        topic: topics::INCOME,
        see_also: &["personal_allowance"],
    },
    // --- capital gains ------------------------------------------------------
    GlossaryEntry {
        id: "capital_gains_tax",
        term: "Capital gains tax",
        also: "CGT",
        definition: "Charged on the growth in an unwrapped holding when you \
                     sell, not on the whole amount you take. Selling part of a \
                     holding realises the same proportion of its gain, which is \
                     how this projection prices a monthly withdrawal.",
        seen_in: "The tax column for an unwrapped holding.",
        topic: topics::GAINS,
        see_also: &["cost_basis", "annual_exempt_amount", ids::GIA],
    },
    GlossaryEntry {
        id: "annual_exempt_amount",
        term: "Annual exempt amount",
        also: "CGT allowance",
        definition: "A slice of gains charged at nothing each year. It resets \
                     yearly and cannot be carried forward, so a gain left \
                     unrealised does not bank the allowance it did not use.",
        seen_in: "The unclaimed-allowance column in the drawdown comparison.",
        topic: topics::GAINS,
        see_also: &["capital_gains_tax", "unused_allowance"],
    },
    GlossaryEntry {
        id: "cost_basis",
        term: "Cost",
        also: "Acquisition cost, base cost",
        definition: "What the holding cost you, as against what it is worth \
                     today. The difference is the gain, and only the gain is \
                     charged -- which is why this box appears only for the \
                     account kinds taxed that way.",
        seen_in: "The cost box beside the account picker.",
        topic: topics::GAINS,
        see_also: &["capital_gains_tax", ids::GIA],
    },
    // --- pensions -----------------------------------------------------------
    GlossaryEntry {
        id: "tax_free_cash",
        term: "Tax-free cash",
        also: "PCLS, pension commencement lump sum, the 25%",
        definition: "A fixed share of each pension withdrawal that carries no \
                     tax, with the rest charged as income. It is taken \
                     alongside every withdrawal here rather than as one lump at \
                     the start, and it stops once the lump sum allowance is \
                     used up.",
        seen_in: "The tax column for a pension, which is lower than the income \
                  rate alone would give.",
        topic: topics::RETIREMENT,
        see_also: &["lump_sum_allowance", ids::SIPP, "marginal_rate"],
    },
    GlossaryEntry {
        id: "lump_sum_allowance",
        term: "Lump sum allowance",
        also: "LSA",
        definition: "A lifetime cap on how much tax-free cash you can take \
                     across all pensions. Unlike the yearly allowances it never \
                     resets, so a long drawdown eventually exhausts it and \
                     later withdrawals are charged in full.",
        seen_in: "A pension's tax rising partway through a long drawdown.",
        topic: topics::RETIREMENT,
        see_also: &["tax_free_cash"],
    },
    GlossaryEntry {
        id: "minimum_access_age",
        term: "Minimum access age",
        also: "Normal minimum pension age, NMPA",
        definition: "The age before which a pension cannot normally be touched. \
                     A projection that would draw on one earlier is refused and \
                     told so, rather than quietly taking the money.",
        seen_in: "The error message naming the holding and the age.",
        topic: topics::RETIREMENT,
        see_also: &[ids::SIPP, ids::WORKPLACE_DC],
    },
    GlossaryEntry {
        id: "unused_allowance",
        term: "Allowance left unclaimed",
        also: "",
        definition: "Allowance a year offered and the withdrawals did not use. \
                     Yearly allowances do not carry forward, so this is the \
                     column that explains why one way of drawing money down \
                     costs less tax than another.",
        seen_in: "The unclaimed-allowance column in the drawdown comparison.",
        topic: topics::RETIREMENT,
        see_also: &["personal_allowance", "annual_exempt_amount"],
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::UK_ACCOUNTS;
    use std::collections::HashSet;

    #[test]
    fn ids_are_unique_and_every_entry_says_something() {
        let mut seen = HashSet::new();
        for e in UK_GLOSSARY {
            assert!(!e.id.is_empty(), "an entry has no id");
            assert!(seen.insert(e.id), "duplicate glossary id {}", e.id);
            assert!(!e.term.is_empty(), "{} has no term", e.id);
            assert!(!e.definition.is_empty(), "{} has no definition", e.id);
            assert!(!e.topic.is_empty(), "{} has no topic", e.id);
        }
    }

    #[test]
    fn every_cross_reference_resolves() {
        let ids: HashSet<_> = UK_GLOSSARY.iter().map(|e| e.id).collect();
        for e in UK_GLOSSARY {
            for r in e.see_also {
                assert!(ids.contains(r), "{} points at missing entry {}", e.id, r);
                assert_ne!(*r, e.id, "{} points at itself", e.id);
            }
        }
    }

    /// A topic that reappears after another has started renders as two
    /// sections with the same heading, which reads as a mistake.
    #[test]
    fn topics_are_contiguous() {
        let mut started: Vec<&str> = Vec::new();
        for e in UK_GLOSSARY {
            if started.last() != Some(&e.topic) {
                assert!(
                    !started.contains(&e.topic),
                    "topic {:?} resumes after another",
                    e.topic
                );
                started.push(e.topic);
            }
        }
    }

    /// The coverage rule: an account kind a holding can sit in must be
    /// explained, and the entry explaining it is found by sharing its id.
    #[test]
    fn every_account_kind_is_explained() {
        let ids: HashSet<_> = UK_GLOSSARY.iter().map(|e| e.id).collect();
        for k in UK_ACCOUNTS {
            assert!(
                ids.contains(k.id),
                "account kind {} ({}) has no glossary entry",
                k.id,
                k.label
            );
        }
    }
}
