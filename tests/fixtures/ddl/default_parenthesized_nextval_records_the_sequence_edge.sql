-- The consequence that makes the parenthesized `nextval` normalization load-bearing
-- rather than cosmetic: reducing `DEFAULT (nextval('s'))` onto the closed `NextVal`
-- arm is what records the column's `pg_attrdef` → sequence NORMAL edge, so the
-- sequence cannot be dropped out from under the default.
--
-- Left as a general expression instead, the edge would not exist: `DROP SEQUENCE`
-- would succeed and every subsequent write to the column would name a relation the
-- catalog lacks — the shape a dump then emits with no `CREATE SEQUENCE` beside it.
-- Measured on 18.6: `2BP01` for the parenthesized spelling exactly as for the bare
-- one, and the `CASCADE` polarity succeeds on both.
-- @expect-error: the parenthesized nextval still blocks the drop (2BP01)
-- @setup
CREATE SEQUENCE dpe_seq;
CREATE TABLE dpe (a INT DEFAULT (nextval('dpe_seq')));
-- @test
DROP SEQUENCE dpe_seq;
