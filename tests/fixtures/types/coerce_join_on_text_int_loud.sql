-- Resolving `ON`'s literals must not make the clause permissive. `ja.t = 5` compares a
-- text column to an integer *constant* — not an unknown literal, so there is nothing
-- to resolve and Postgres raises `42883` (`operator does not exist: text = integer`).
-- The clause-coercion pass runs the same static operand gate here that it runs on the
-- projection and `WHERE`, so the rejection now lands at plan time rather than
-- depending on a row pair matching.
-- @setup
CREATE TABLE ja (id INT, t TEXT);
CREATE TABLE jb (id INT, u TEXT);
INSERT INTO ja VALUES (1, 'x'), (2, 'y');
INSERT INTO jb VALUES (1, 'p'), (2, 'q');
-- @test
-- @expect-error: no text = integer operator in a JOIN ON
SELECT ja.t AS jt FROM ja JOIN jb ON ja.id = jb.id AND ja.t = 5;
