//! What Germany's own words mean, as [`taxkit::GlossaryEntry`] data.
//!
//! **Data, like [`crate::tables`] -- not mechanism.** These are the terms this
//! crate puts on screen, and they carry more weight here than they do for a
//! jurisdiction whose vocabulary a reader may already share: an English-reading
//! user meets `Vorabpauschale` in a tax column with no way to guess what it is.
//!
//! Each entry keeps the German term as `term` and gives the English gloss in
//! `also`, rather than the reverse. The German word is what appears elsewhere in
//! the interface, and a glossary that renamed it would stop being a way to look
//! up what you are seeing.
//!
//! Two disciplines, both inherited from `AccountKind::note`:
//!
//! * **Descriptive, never advisory.** Say what a rule does and which figure it
//!   moves. Never what a reader should do about it.
//! * **No rates, no thresholds.** Figures live in [`crate::tables`] and change
//!   yearly; a definition that quotes one is a second place to update and a
//!   silent way to go stale. Name the allowance, do not price it.
//!
//! Every account kind in [`crate::tables::DE_ACCOUNTS`] has an entry whose `id`
//! is that kind's id, which is what the coverage test checks -- so a new
//! account kind cannot ship unexplained.

use taxkit::GlossaryEntry;

use crate::tables::ids;

/// Topic labels, so the grouping reads the same in every entry.
mod topics {
    pub const ACCOUNTS: &str = "Konten und Anlagen";
    pub const CAPITAL: &str = "Kapitalerträge";
    pub const INCOME: &str = "Einkommensteuer";
    pub const RETIREMENT: &str = "Altersvorsorge";
}

/// The German glossary, in presentation order and contiguous by topic.
pub const DE_GLOSSARY: &[GlossaryEntry] = &[
    // --- accounts -----------------------------------------------------------
    GlossaryEntry {
        id: ids::GIRO,
        term: "Giro-/Tagesgeldkonto",
        also: "Current or instant-access account",
        definition: "Cash held at a bank. Interest would be capital income, but \
                     this projection models total return rather than yield, so a \
                     cash holding is carried untaxed and nothing is charged when \
                     you take from it.",
        seen_in: "The account column, and the zero in the tax column beside it.",
        topic: topics::ACCOUNTS,
        see_also: &["abgeltungsteuer"],
    },
    GlossaryEntry {
        id: ids::DEPOT_AKTIEN,
        term: "Aktien-/Anleihedepot",
        also: "Share and bond account, direct holdings",
        definition: "Shares or bonds held directly rather than through a fund. \
                     The gain is charged the flat Abgeltungsteuer when you sell, \
                     with no Teilfreistellung and no yearly holding charge.",
        seen_in: "The cost box beside the account picker, and the tax column.",
        topic: topics::ACCOUNTS,
        see_also: &["abgeltungsteuer", "teilfreistellung", "cost_basis"],
    },
    GlossaryEntry {
        id: ids::FONDS_AKTIEN,
        term: "Aktienfonds / Aktien-ETF",
        also: "Equity fund or ETF",
        definition: "A fund holding mostly shares. Part of its gain is exempt \
                     through the Teilfreistellung, and it carries the yearly \
                     Vorabpauschale whether or not you sell anything.",
        seen_in: "The tax column, and the charge column during accumulation.",
        topic: topics::ACCOUNTS,
        see_also: &["teilfreistellung", "vorabpauschale", "abgeltungsteuer"],
    },
    GlossaryEntry {
        id: ids::FONDS_MISCH,
        term: "Mischfonds",
        also: "Mixed or balanced fund",
        definition: "A fund holding a mixture of shares and other assets. The \
                     same rules as an equity fund, with a smaller exempt share \
                     because less of it is in shares.",
        seen_in: "The tax column.",
        topic: topics::ACCOUNTS,
        see_also: &[ids::FONDS_AKTIEN, "teilfreistellung"],
    },
    GlossaryEntry {
        id: ids::FONDS_IMMO,
        term: "Immobilienfonds",
        also: "Property fund",
        definition: "A fund holding property. Again the same rules, with its own \
                     exempt share reflecting what the fund holds.",
        seen_in: "The tax column.",
        topic: topics::ACCOUNTS,
        see_also: &[ids::FONDS_AKTIEN, "teilfreistellung"],
    },
    GlossaryEntry {
        id: ids::PRIVATE_RV,
        term: "Private Rentenversicherung (sonst)",
        also: "Private annuity contract, outside the 12/62 rule",
        definition: "A private retirement contract that does not meet the 12/62 \
                     conditions. Its gain is charged the flat Abgeltungsteuer, \
                     like any other capital holding.",
        seen_in: "The account column, and the tax column.",
        topic: topics::ACCOUNTS,
        see_also: &["zwoelf_zweiundsechzig", ids::PRIVATE_RV_12, "abgeltungsteuer"],
    },
    GlossaryEntry {
        id: ids::PRIVATE_RV_12,
        term: "Private Rentenversicherung (12 J. / ab 62)",
        also: "Private annuity contract meeting the 12/62 rule",
        definition: "The same contract, once it has run twelve years and the \
                     holder is past 62. Half the gain is then charged at the \
                     personal income-tax rate instead of the flat rate, which is \
                     why it is a separate account kind rather than a setting.",
        seen_in: "The account picker, where you choose which of the two applies.",
        topic: topics::ACCOUNTS,
        see_also: &["zwoelf_zweiundsechzig", ids::PRIVATE_RV, "tarif"],
    },
    GlossaryEntry {
        id: ids::RUERUP,
        term: "Rürup-Rente (Basisrente)",
        also: "Basic retirement provision",
        definition: "A retirement contract relieved on the way in and charged as \
                     income on the way out, at a taxable share fixed by the year \
                     the payments begin. It cannot be reached before its minimum \
                     age.",
        seen_in: "The tax column, and the year-drawing-starts control.",
        topic: topics::ACCOUNTS,
        see_also: &["besteuerungsanteil", "mindestalter", "tarif"],
    },
    GlossaryEntry {
        id: ids::BAV,
        term: "Betriebliche Altersvorsorge",
        also: "bAV, occupational provision",
        definition: "Retirement provision through an employer. Charged as income \
                     in full when it is paid, and likewise age-gated.",
        seen_in: "The tax column.",
        topic: topics::ACCOUNTS,
        see_also: &["tarif", "mindestalter"],
    },
    GlossaryEntry {
        id: ids::RIESTER,
        term: "Riester-Rente",
        also: "Subsidised private provision",
        definition: "A subsidised retirement contract whose allowances, clawbacks \
                     and payout rules are not modelled here. It is selectable \
                     because portfolios really hold one, but its figures are \
                     projected untaxed and should be read as indicative only.",
        seen_in: "The account column, and the note that says it is not modelled.",
        topic: topics::ACCOUNTS,
        see_also: &[ids::RUERUP],
    },
    GlossaryEntry {
        id: ids::IMMOBILIE,
        term: "Immobilie (direkt)",
        also: "Property held directly",
        definition: "Property owned outright rather than through a fund. Its own \
                     rules \u{2014} the ten-year holding period, owner-occupation, \
                     depreciation \u{2014} are not modelled, so it is carried untaxed.",
        seen_in: "The account column, and the note that says it is not modelled.",
        topic: topics::ACCOUNTS,
        see_also: &[ids::FONDS_IMMO],
    },
    // --- capital income -----------------------------------------------------
    GlossaryEntry {
        id: "abgeltungsteuer",
        term: "Abgeltungsteuer",
        also: "Flat tax on capital income",
        definition: "Capital gains are charged at one flat rate rather than at \
                     your personal income-tax rate, with the Solidaritäts\
                     zuschlag and any Kirchensteuer added on top. The rate does \
                     not rise with the amount you take, which is what makes a \
                     capital withdrawal behave so differently from a pension one \
                     in the drawdown comparison.",
        seen_in: "The tax column for every depot and fund holding.",
        topic: topics::CAPITAL,
        see_also: &["sparer_pauschbetrag", "soli", "kirchensteuer"],
    },
    GlossaryEntry {
        id: "sparer_pauschbetrag",
        term: "Sparer-Pauschbetrag",
        also: "Saver's allowance",
        definition: "A slice of capital income charged at nothing each year. It \
                     resets yearly and cannot be carried forward, and the \
                     Vorabpauschale draws on the same allowance \u{2014} so a charge \
                     during accumulation can leave less of it for a withdrawal \
                     later in the same year.",
        seen_in: "The unclaimed-allowance column in the drawdown comparison.",
        topic: topics::CAPITAL,
        see_also: &["abgeltungsteuer", "vorabpauschale"],
    },
    GlossaryEntry {
        id: "teilfreistellung",
        term: "Teilfreistellung",
        also: "Partial exemption",
        definition: "A fixed share of a fund's gain is exempt, in rough \
                     compensation for tax the fund has already paid internally. \
                     The share depends on what the fund holds, so an equity fund \
                     keeps more of a gain than a direct holding of the same size.",
        seen_in: "A fund's tax column being lower than a depot's for the same gain.",
        topic: topics::CAPITAL,
        see_also: &[ids::FONDS_AKTIEN, "abgeltungsteuer"],
    },
    GlossaryEntry {
        id: "vorabpauschale",
        term: "Vorabpauschale",
        also: "Advance lump-sum charge",
        definition: "A yearly charge on a fund you have not sold, calculated \
                     from its value at the start of the year and a rate set each \
                     January. It is the reason a German accumulation projection \
                     shows tax being paid while nothing is being withdrawn, and \
                     it is capped so that a fund which fell is charged nothing.",
        seen_in: "The tax-charged card, and the charge column in the breakdown.",
        topic: topics::CAPITAL,
        see_also: &["basiszins", "sparer_pauschbetrag", ids::FONDS_AKTIEN],
    },
    GlossaryEntry {
        id: "basiszins",
        term: "Basiszins",
        also: "Base rate for the Vorabpauschale",
        definition: "The rate the Vorabpauschale is worked out from, published \
                     once a year and applied to the fund's opening value. When it \
                     is low, so is the charge.",
        seen_in: "The size of the yearly charge on a fund holding.",
        topic: topics::CAPITAL,
        see_also: &["vorabpauschale"],
    },
    GlossaryEntry {
        id: "cost_basis",
        term: "Anschaffungskosten",
        also: "Cost, what the holding cost you",
        definition: "What you paid, as against what the holding is worth today. \
                     The difference is the gain, and only the gain is charged \u{2014} \
                     which is why this box appears only for the account kinds \
                     taxed that way.",
        seen_in: "The cost box beside the account picker.",
        topic: topics::CAPITAL,
        see_also: &["abgeltungsteuer", ids::DEPOT_AKTIEN],
    },
    // --- income tax ---------------------------------------------------------
    GlossaryEntry {
        id: "tarif",
        term: "Einkommensteuertarif (§ 32a EStG)",
        also: "The income-tax tariff",
        definition: "Germany's income tax rises continuously rather than in \
                     steps: within the middle zones each extra euro is charged a \
                     little more than the last, instead of jumping at a band \
                     edge. Retirement income is charged this way, capital income \
                     is not.",
        seen_in: "The tax column for a Rürup or bAV withdrawal.",
        topic: topics::INCOME,
        see_also: &["grundfreibetrag", "splitting", "abgeltungsteuer"],
    },
    GlossaryEntry {
        id: "grundfreibetrag",
        term: "Grundfreibetrag",
        also: "Basic allowance",
        definition: "The slice of income charged at nothing before the tariff \
                     starts. It resets each year, so spreading retirement income \
                     across years uses more of it than taking it in one.",
        seen_in: "The unclaimed-allowance column in the drawdown comparison.",
        topic: topics::INCOME,
        see_also: &["tarif", "unused_allowance"],
    },
    GlossaryEntry {
        id: "soli",
        term: "Solidaritätszuschlag",
        also: "Soli, solidarity surcharge",
        definition: "A surcharge charged on the tax itself rather than on \
                     income, and only above a threshold, with a phase-in stretch \
                     just above it where it climbs quickly. It applies to both \
                     the tariff and the flat capital rate.",
        seen_in: "Every tax figure, folded into it rather than shown separately.",
        topic: topics::INCOME,
        see_also: &["tarif", "abgeltungsteuer", "kirchensteuer"],
    },
    GlossaryEntry {
        id: "kirchensteuer",
        term: "Kirchensteuer",
        also: "Church tax",
        definition: "A further charge on the tax itself, for members of a \
                     church that levies it, at a rate that depends on the state. \
                     It is chosen here through the region picker, which conflates \
                     the state with membership \u{2014} pick \u{201c}no church tax\u{201d} if you are \
                     not a member.",
        seen_in: "The region picker, and every tax figure.",
        topic: topics::INCOME,
        see_also: &["soli", "abgeltungsteuer"],
    },
    GlossaryEntry {
        id: "splitting",
        term: "Splittingverfahren",
        also: "Joint assessment, Ehegattensplitting",
        definition: "A married couple assessed together are charged twice the \
                     tax on half their joint income, which softens the \
                     progression considerably. Selecting it changes what \
                     retirement income costs, and leaves the flat capital rate \
                     alone.",
        seen_in: "The assessment control in the tax settings.",
        topic: topics::INCOME,
        see_also: &["tarif", "other_income"],
    },
    GlossaryEntry {
        id: "other_income",
        term: "Übrige steuerpflichtige Einkünfte",
        also: "Other taxable income",
        definition: "Income you already expect to receive \u{2014} the state pension, \
                     rent, work. It is not part of the pot, but it is charged \
                     first, so withdrawals on top of it start further up the \
                     tariff.",
        seen_in: "The tax settings, and every taxed withdrawal figure.",
        topic: topics::INCOME,
        see_also: &["tarif", "splitting"],
    },
    GlossaryEntry {
        id: "unused_allowance",
        term: "Nicht genutzter Freibetrag",
        also: "Allowance left unclaimed",
        definition: "Allowance a year offered and the withdrawals did not use. \
                     Yearly allowances do not carry forward, so this is the \
                     column that explains why one way of drawing money down \
                     costs less tax than another.",
        seen_in: "The unclaimed-allowance column in the drawdown comparison.",
        topic: topics::INCOME,
        see_also: &["grundfreibetrag", "sparer_pauschbetrag"],
    },
    // --- retirement ---------------------------------------------------------
    GlossaryEntry {
        id: "besteuerungsanteil",
        term: "Besteuerungsanteil",
        also: "Taxable share, fixed by cohort",
        definition: "The share of a Rürup payment that is charged as income. It \
                     is set by the year the payments start and then stays that \
                     way for life, so starting a year later can fix a different \
                     share for the whole of a long drawdown.",
        seen_in: "The year-drawing-starts control, and a Rürup's tax column.",
        topic: topics::RETIREMENT,
        see_also: &[ids::RUERUP, "tarif"],
    },
    GlossaryEntry {
        id: "zwoelf_zweiundsechzig",
        term: "12/62-Regel",
        also: "The 12/62 rule",
        definition: "A private retirement contract held twelve years and paid \
                     out after 62 has half its gain charged at the personal rate \
                     rather than the whole gain at the flat rate. Whether it \
                     applies is a yes-or-no fact about the contract, so it is \
                     offered as two account kinds rather than a switch.",
        seen_in: "The two Private Rentenversicherung entries in the account picker.",
        topic: topics::RETIREMENT,
        see_also: &[ids::PRIVATE_RV_12, ids::PRIVATE_RV],
    },
    GlossaryEntry {
        id: "mindestalter",
        term: "Mindestalter",
        also: "Minimum access age",
        definition: "The age before which a retirement holding cannot normally \
                     be touched, which differs by kind. A projection that would \
                     draw on one earlier is refused and told so, rather than \
                     quietly taking the money.",
        seen_in: "The error message naming the holding and the age.",
        topic: topics::RETIREMENT,
        see_also: &[ids::RUERUP, ids::BAV],
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::DE_ACCOUNTS;
    use std::collections::HashSet;

    #[test]
    fn ids_are_unique_and_every_entry_says_something() {
        let mut seen = HashSet::new();
        for e in DE_GLOSSARY {
            assert!(!e.id.is_empty(), "an entry has no id");
            assert!(seen.insert(e.id), "duplicate glossary id {}", e.id);
            assert!(!e.term.is_empty(), "{} has no term", e.id);
            assert!(!e.definition.is_empty(), "{} has no definition", e.id);
            assert!(!e.topic.is_empty(), "{} has no topic", e.id);
        }
    }

    #[test]
    fn every_cross_reference_resolves() {
        let ids: HashSet<_> = DE_GLOSSARY.iter().map(|e| e.id).collect();
        for e in DE_GLOSSARY {
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
        for e in DE_GLOSSARY {
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
        let ids: HashSet<_> = DE_GLOSSARY.iter().map(|e| e.id).collect();
        for k in DE_ACCOUNTS {
            assert!(
                ids.contains(k.id),
                "account kind {} ({}) has no glossary entry",
                k.id,
                k.label
            );
        }
    }

    /// The German term is what the rest of the interface shows, so it is the
    /// thing a reader looks up. An entry that led with the English gloss would
    /// not be findable from the screen.
    #[test]
    fn account_entries_keep_the_catalogue_wording() {
        for k in DE_ACCOUNTS {
            let e = DE_GLOSSARY.iter().find(|e| e.id == k.id).unwrap();
            assert_eq!(e.term, k.label, "{} renames the account it explains", k.id);
        }
    }
}
