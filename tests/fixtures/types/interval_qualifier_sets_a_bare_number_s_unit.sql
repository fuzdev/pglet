-- `ConstInterval Sconst opt_interval`. Two rules, in order: a **bare number**
-- literal takes the qualifier's unit (`DecodeInterval`'s `range` argument), and the
-- result is then masked to that field (`AdjustIntervalForTypmod`) — fields to the
-- right of the named one are zeroed, those to its left kept.
--
-- Unparsed, the qualifier was read as an output alias and `INTERVAL '90' MINUTE`
-- answered ninety *seconds*: a wrong value, not a parse wall.
-- @test
SELECT
  INTERVAL '1' DAY AS d,
  INTERVAL '90' MINUTE AS m,
  INTERVAL '1' YEAR AS y,
  INTERVAL '1' MONTH AS mo,
  INTERVAL '1' HOUR AS h,
  INTERVAL '90' SECOND AS s,
  INTERVAL '1.5' SECOND AS frac,
  INTERVAL '-1' DAY AS neg;
