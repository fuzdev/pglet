-- A `varchar(n)` column enforces its length on store: a value within `n` is kept
-- verbatim, and an over-length value whose excess is all blanks (spaces) is
-- silently truncated to `n` characters (Postgres `varchar_input`), rather than
-- erroring. Non-blank excess is the loud case (see `varchar_length_overflow_loud`).
-- @setup
CREATE TABLE vc (id INT, v varchar(3));
INSERT INTO vc VALUES (1, 'abc');
INSERT INTO vc VALUES (2, 'abc  ');
INSERT INTO vc VALUES (3, 'ab');
-- @test
SELECT id, v FROM vc ORDER BY id;
