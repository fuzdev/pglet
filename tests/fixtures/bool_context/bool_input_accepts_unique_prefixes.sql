-- `boolin` is a prefix grammar, not a closed list of twelve spellings: any
-- case-insensitive non-empty prefix of true/false/yes/no, `on`/`off` from two
-- characters up, and `1`/`0` at length exactly one. `n_` and `on_` spell members
-- of that old closed list and were accepted here already; `tr`, `tru`, `tr_upper`,
-- `fals`, `fal_upper`, `fa`, `ye`, `of_` and `of_upper` were `22P02`.
-- @test
SELECT
  'tr'::bool AS tr, 'tru'::bool AS tru, 'Tr'::bool AS tr_upper,
  'fals'::bool AS fals, 'FAL'::bool AS fal_upper, 'fa'::bool AS fa,
  'ye'::bool AS ye, 'n'::bool AS n_,
  'of'::bool AS of_, 'OF'::bool AS of_upper, 'on'::bool AS on_;
