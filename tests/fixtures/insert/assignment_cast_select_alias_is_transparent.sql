-- **A false-rejection control — do not trim it.** An `AS` label lives on the target
-- entry beside the expression, not inside it, so `SELECT 't' AS z` carries the same
-- `unknown` the bare spelling does and 18.6 assigns it into a `BOOL` column.
--
-- The unknown carve-out reads the expression's *shape*, so the alias has to be peeled
-- explicitly; drop that arm and this statement becomes a spurious `42804` while the
-- unaliased fixtures beside it stay green.
-- @setup
CREATE TABLE acal (id INT, b BOOL, i INT);
-- @test
INSERT INTO acal (b, i) SELECT 't' AS z, '5' AS y RETURNING b, i;
