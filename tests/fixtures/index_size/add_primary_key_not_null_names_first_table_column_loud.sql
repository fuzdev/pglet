-- The NOT NULL scan reads a row's columns in the table's order, not the key's: over
-- `(NULL, NULL)`, `PRIMARY KEY (b, a)` names `a`.
-- @setup
CREATE TABLE t (a text, b text);
INSERT INTO t VALUES (NULL, NULL);
-- @test
-- @expect-error: the first NULL column in table order
-- @expect-message
-- @profile-lock
ALTER TABLE t ADD PRIMARY KEY (b, a);
