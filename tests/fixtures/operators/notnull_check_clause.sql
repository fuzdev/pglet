-- A stored `NOTNULL` / `ISNULL` is the null test, rendered as 18.6 renders it.
-- @setup
CREATE TABLE nck (a INT CHECK (a NOTNULL), b INT CHECK (b ISNULL OR b > 0));
-- @test
-- @no-declared-types: pglet declares these information_schema columns `text` where 18.6 declares `name` and `varchar`; the subject is the rendered clause
SELECT constraint_name, check_clause FROM information_schema.check_constraints
WHERE constraint_name LIKE 'nck%' ORDER BY constraint_name;
