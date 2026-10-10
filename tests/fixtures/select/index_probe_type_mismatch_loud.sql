-- **Build-profile regression lock.** An accelerated `=` becomes an index probe and
-- never reaches the operator's eval-time gate, so the probe value has to be gated
-- where it is coerced to the key column's type. Ungated it *reinterpreted* the
-- mismatch — `= true` against an `int4` key coerced to `= 1` and returned the
-- matching row — while the stripped (`--no-default-features`) build ran
-- `Filter(Scan)` and returned none. That broke the load-bearing invariant that an
-- index changes only speed, never a result. Both profiles must now raise `42883`.
-- @profile-lock
-- @setup
CREATE TABLE ix_probe (n INT, label TEXT);
CREATE INDEX ix_probe_n ON ix_probe (n);
INSERT INTO ix_probe VALUES (1, 'one'), (0, 'zero');
-- @test
-- @expect-error: no integer = boolean operator, on both build profiles
SELECT * FROM ix_probe WHERE n = true;
