-- The `INSERT … SELECT` spelling of the same gate, and the one shape that was already
-- half-covered: the row source lowers each value into an `Expr::Bound`, so the store
-- finalizer *did* see an `int4` heading for a `BOOL` column — but only per row, so an
-- **empty** source reported success where 18.6 refuses, and the wider pairs the
-- finalizer's two-pair list does not carry (this file's `text → integer` half) were
-- performed silently.
--
-- The source is deliberately empty: the answer must not depend on whether a row exists.
-- @setup
CREATE TABLE acsel (id INT, b BOOL, i INT);
CREATE TABLE acsrc (n INT, t TEXT);

-- @expect-error: column "b" is of type boolean but expression is of type integer (42804)
-- @test
INSERT INTO acsel (b) SELECT n FROM acsrc;
