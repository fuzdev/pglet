-- `RENAME CONSTRAINT` is refused `0A000` here; 18.7 renames the constraint, so any error passes.
-- @profile-lock
-- @setup
CREATE TABLE cp (id serial PRIMARY KEY, v text UNIQUE, n int CHECK (n > 0), m int, UNIQUE (m, n));
CREATE INDEX cp_lower ON cp (lower(v)) WHERE n > 1;
CREATE TABLE cc (pid int REFERENCES cp (id), pm int, pn int, FOREIGN KEY (pm, pn) REFERENCES cp (m, n));
INSERT INTO cp (v, n, m) VALUES ('a', 5, 1), ('b', 7, 2);
-- @expect-error: ALTER TABLE … RENAME CONSTRAINT is not supported
-- @test
ALTER TABLE cp RENAME CONSTRAINT cp_n_check TO cp_positive;
