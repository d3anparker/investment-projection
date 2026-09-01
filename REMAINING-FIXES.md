# Still to fix

Five things still to change. Four came out of the code review; number 3 you
spotted yourself. Each one needs you to decide something, so none was mine to
just fix.

They are in order of how much they matter.

---

## 1. Germany charges no tax at all on shares if you have no other income

**Where:** `de-tax/src/engine.rs`, the `capital_rate` function.

**What happens now**

Germany has two ways to tax money you make from shares and funds:

- a flat rate of about 26%, or
- your normal income tax rate, if that works out cheaper for you.

The code picks whichever is cheaper. That part is right.

The problem is *how* it works out your normal income tax rate. It looks at the
money you already have coming in, and ignores the share money you are about to
take out.

So if you have no other income, it looks at zero. Tax on zero income is zero.
So it decides your normal rate is 0%, and 0% is cheaper than 26%, so it charges
you nothing.

And it charges you nothing *no matter how much you take out*.

**Proof**

I ran it. Someone with no other income takes 500,000 euros of pure profit out
of a share account. The tool says the tax is **zero euros**.

That is not a rounding problem. That is the whole tax bill missing.

**Why this is bad**

The app's boxes start empty. "Other income" starts blank, which means zero. So
this is not a weird corner case — it is what happens the first time anyone tries
Germany.

**Why I did not fix it**

There is no small fix. Two options, both need a real decision from you:

1. **Count the share money as income as you take it out.** Then the rate climbs
   as the year goes on, which is closer to the truth. But that money would then
   start eating the tax-free income allowance, which changes how pensions are
   taxed too. That is a knock-on effect you should choose on purpose, not have
   me sneak in.

2. **Build the rising rate into the price list properly.** This is the "right"
   answer but the tool that holds price lists can only handle flat steps, and
   Germany's rate climbs smoothly. It would need new machinery.

---

## 2. Germany's yearly fund charge treats your savings as profit

**Where:** `de-tax/src/engine.rs`, the line that works out `gain`.

**What happens now**

Germany charges a small tax each year on funds you are just *holding*, even if
you sell nothing. There is a fair rule attached: the charge can never be bigger
than how much the fund actually went up that year. If the fund went down, you
pay nothing.

The code works out "how much it went up" by taking the value at the end of the
year and subtracting the value at the start.

That is fine if you left it alone. But if you were paying money in every month,
that money is sitting in the end value too. So the sum thinks your own savings
were profit.

**What goes wrong**

A fund that lost money all year, but that you paid into every month, ends the
year worth more than it started. The code sees that and says "it went up", so it
charges you the tax. In real life you would owe nothing, because the fund fell.

**Why I did not fix it**

To fix it, the fund has to be told how much money was paid into it during the
year. Right now nothing tells it that. Adding that means changing the shared
agreement that sits between the maths engine and every country's tax rules —
which is a bigger change than a review should make on its own.

There is already a note in the code admitting the *opposite* problem happens
when you are taking money out. This one is not mentioned, so at the very least
the note should say both.

---

## 3. The boxes you type into still show £ when Germany is picked

**Where:** `app/styles.css`, the `.adorn-money::before` rule.

**What happens now**

Pick Germany and the *answers* switch to euros — the big numbers, the table, the
chart. That part works.

But the little currency sign sitting inside each box you type into stays a pound
sign. Six boxes: value today, monthly deposit, cost, monthly withdrawal, other
income, and the goal target.

So the screen shows you £ and € at the same time, for the same money.

**Why it was missed**

The pound sign is not in the program. It is in the stylesheet, written as a
decoration glued to the edge of the box:

```
content: "\00a3";
```

`\00a3` is a pound sign, spelled in a way that does not look like one.

Everything else that prints money asks the country which sign to use. This one
never asks anybody — the stylesheet just draws it. So when the country switch was
built, this was not on the list of things to update, because nothing connected it
to the country in the first place.

It also slips past the safety checks. There are automatic checks that hunt for
country-specific things in the wrong place, but they only read program files.
Nothing checks the stylesheet.

**What the fix looks like**

The stylesheet cannot ask the country anything — it has to be told. So the
program needs to hand the sign down to it as a named value, and the rule uses
that value instead of a fixed pound sign.

That is a small change and I could have done it. I have left it here because
there are two decisions attached that are yours, not mine:

1. **Where the sign goes.** Britain writes £5, Germany normally writes 5 €, with
   the sign after the number. Right now everything in the app puts the sign
   first. Matching German habit properly means the box decoration has to be able
   to sit on either side — a bigger change than swapping one character.

2. **Whether the answers should move too.** The output figures also put € first.
   Whatever you choose, the boxes and the answers should agree, so it is one
   decision covering both rather than two half-fixes.

If you would rather just stop the wrong sign showing and settle the placement
question later, say so and I will do the one-character version.

---

## 4. The "unused allowance" number counts years you could never have used

**Where:** `calc/src/engine.rs`, the line that sets `anchor`.

**What happens now**

Everyone gets a chunk of income each year they pay no tax on. If you do not use
all of it, the app adds it to an "allowance unused" total. That total is there
to explain *why* one way of taking your money beats another.

Germany needed the tax year clock to start on day one, because its yearly fund
charge happens while you are still saving up. Fair enough. But turning that
clock on also switched on the allowance counter, all the way through the
saving-up years.

**What goes wrong**

Picture saving for 20 years, then drawing down. During those 20 years you are
taking nothing out, so of course you use none of the allowance. There was
nothing to use it on.

The app counts it anyway. That is 20 years of allowance, roughly a quarter of a
million euros, added to a number that is supposed to mean "money you left on the
table".

Britain does not have this problem, because its clock only starts when you begin
drawing down.

**There is a second problem with the same cause**

Starting the clock on day one also means the tax year no longer lines up with
the day you start drawing down.

Say you save for 30 months, then start drawing. The clock ticks over at month
12, 24, 36. So your first "year" of drawing down is only 6 months long — but it
still hands you a full year's worth of tax-free allowances.

The code has a comment right next to it warning about exactly this. It says
starting the clock at month zero "would manufacture a stub first period carrying
a full year's allowances". Then the German version does it anyway.

**Why I did not fix it**

Two ways to do it and I do not know which you want:

1. The maths engine stops the counter until drawing-down starts.
2. Germany's rules stop counting during the saving-up years.

Option 2 needs Germany's rules to know which phase it is in, and right now they
are not told. So somebody has to decide who owns this.

The stub-year problem has a neat one-line fix — start the clock at
`horizon % 12` instead of `0`, so a tick always lands exactly on the day you
start drawing. But it changes the numbers the app reports, and it has to stay
switched off for Britain or Britain picks up problem one. So it is a decision,
not a tidy-up.

---

## 5. The "year you start drawing" box shows a year it does not use

**Where:** `app/src/jurisdiction/de.rs`.

**What happens now**

Germany has a box asking what year you start taking your pension. It matters,
because the year you start locks in how much of that pension gets taxed, for
life.

The box shows the current year as a starting suggestion. But it only *shows* it.
It never actually hands that year over to the maths. When nothing is handed
over, the maths quietly falls back to whatever year the tax tables were written
for.

**What goes wrong**

Right now both say 2026, so nothing looks wrong. But the tax tables only get
updated once a year. The moment the calendar rolls into 2027 and the tables have
not been refreshed yet, the box on screen says 2027 while the sums behind it use
2026. Different numbers, no warning.

**Why I did not fix it**

I did write the fix, and then took it back out.

The fix makes the box save its suggestion the moment it appears. That save
happens a split second late, and that was enough to confuse a test: one test
would set the country to Germany, finish, and the late save would fire
afterwards and leave Germany switched on for the *next* test, which expected
Britain. That test then failed.

That is worth knowing on its own. The app remembers "which country are we in"
in a single shared spot that anything can read at any time. Nothing guarantees
it is up to date when you read it. My small fix did not create that weakness, it
just tripped over it. It is likely to trip someone else up later.

---

## Summary

| # | Fix | How bad | What is blocking it |
|---|-----|---------|---------------------|
| 1 | No tax on German shares with no other income | Serious | Needs a decision on how to price it |
| 2 | Savings counted as profit for the yearly fund charge | Medium | Needs the shared agreement widened |
| 3 | Input boxes still show £ under Germany | Medium | Small fix, but needs the £5-vs-5 € call |
| 4 | Unused allowance counted during saving-up years | Medium | Needs a decision on who fixes it |
| 5 | Pension start-year box shows one year, uses another | Small | Needs the country memory made safe first |

Everything else the review found is already fixed and tested. All tests pass.
