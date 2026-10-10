-- The half of the `INSERT … SELECT` gate the store finalizer's measured pair list cannot
-- reach: `text → integer` is not one of its six pairs, so a source column of text was
-- run through the integer input function and stored — `22P02` for an unparseable value,
-- and *silently correct-looking* for a parseable one — where 18.6 refuses the statement
-- outright.
-- @setup
CREATE TABLE acselt (id INT, i INT);
CREATE TABLE acsrct (n INT, t TEXT);
INSERT INTO acsrct VALUES (1, '5');

-- @expect-error: column "i" is of type integer but expression is of type text (42804)
-- @test
INSERT INTO acselt (i) SELECT t FROM acsrct;
