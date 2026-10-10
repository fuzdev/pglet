-- A value function **shadows a same-named column**, and both engines do it: the word
-- is a keyword to the scanner, so the bare spelling is the session function and only
-- the quoted spelling reaches the column. Pinned over a `"user"` column, which is the
-- one spelling of this shape 18.6 will let a `CREATE TABLE` declare at all.
--
-- What is *not* pinned here, because no fixture can be: pglet also accepts the
-- **unquoted** `CREATE TABLE uq (user TEXT)` where 18.6 is `42601` — pglet accepts
-- where the oracle errors, which `@expect-error` cannot express. `EDGES.md` row 75.
--
-- The two `SELECT`s must disagree, which is what the second row is for: over a table
-- whose stored value happened to equal the connecting role, a reader that returned
-- the column would pass.
-- @setup
CREATE TABLE uq_shadow (id INT, "user" TEXT);
INSERT INTO uq_shadow VALUES (1, 'zeta'), (2, 'postgres');
-- @test
SELECT id, user AS role, "user" AS col, user = "user" AS same FROM uq_shadow ORDER BY id;
