-- The accepting side of `interval_multiple_bare_fields_is_22007.sql`, and the reason
-- the refusal has to count unlabelled fields rather than reject a space: every one of
-- these carries **at most one** of them, and 18.6 answers each with a value. `'1 2:30'`
-- is the sharp one — a bare number followed by a *time* field reads as days, so the
-- bare field is still single.
-- @test
SELECT
  INTERVAL '1' AS lone,
  INTERVAL '1 day 2' AS trailing_bare,
  INTERVAL '1-2 3' AS after_year_month,
  INTERVAL '1 2:30' AS before_a_time,
  INTERVAL '1 04:05:06' AS before_a_full_time,
  INTERVAL '1 day 3 hours' AS all_labelled;
