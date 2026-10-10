-- The `ORDER BY` output-name allowance belongs to the **key**, not to the names inside
-- it. `findTargetlistEntrySQL92` is entered only for a key that *is* a one-field
-- `ColumnRef`, so `q+0` is an ordinary expression whose `q` `transformExpr` resolves
-- against the range table — the base column, not the output alias that shadows it.
--
-- `a AS q` shadows the base `q`, and the two run opposite, so the orderings are
-- opposite and the divergence is a wrong row order with no error on either engine:
-- 18.6 answers `200 300 100` (base `q` = 3,1,2) and pglet answered `100 200 300`
-- (output `q` = the projected `a`).
--
-- Its control is `order_by_nested_name_parenthesized.sql`: parentheses do not build a
-- node in Postgres's grammar, so `ORDER BY (q)` is still a one-field `ColumnRef` and
-- still takes the alias. The pair is what says the split is the key's parse shape and
-- not a textual "does the key contain anything besides a name".
-- @setup
CREATE TABLE onb (q INT, a INT);
INSERT INTO onb VALUES (3, 100), (1, 200), (2, 300);

-- @test
SELECT a AS q FROM onb ORDER BY q+0;
