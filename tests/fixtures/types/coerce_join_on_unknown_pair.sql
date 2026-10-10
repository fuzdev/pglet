-- Two unknown literals in a `JOIN … ON` have no concrete side to adopt, so both stay
-- text and `'a' = 'b'` is a legal false text comparison — the join yields no rows and
-- raises no error, matching Postgres. This guards the pass against over-resolving:
-- the unknown-literal rule needs a *typed* counterpart, and inventing one here would
-- turn a 0-row answer into an error.
-- @setup
CREATE TABLE ja (id INT, t TEXT);
CREATE TABLE jb (id INT, u TEXT);
INSERT INTO ja VALUES (1, 'x'), (2, 'y');
INSERT INTO jb VALUES (1, 'p'), (2, 'q');
-- @test
SELECT ja.t AS jt FROM ja JOIN jb ON ja.id = jb.id AND 'a' = 'b' ORDER BY jt;
