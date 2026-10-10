-- Every CHECK is reported, including the unnamed ones. `information_schema` skipped a
-- check with no stored name entirely, so a table with three constraints introspected as
-- a table with none — silently, since there is nothing in the output to notice. The
-- names follow Postgres's rule: a predicate over exactly one column is
-- `<table>_<col>_check` (the second such collides and takes `1`), one over several is
-- `<table>_check`.
-- @setup
CREATE TABLE cantc (a INT, b INT, CHECK (a > 0), CHECK (a < 100), CHECK (a + b > 0));

-- @test
SELECT constraint_name::text AS constraint_name, constraint_type::text AS constraint_type
FROM information_schema.table_constraints
WHERE table_schema = 'public' AND table_name = 'cantc' AND constraint_type = 'CHECK'
ORDER BY constraint_name;
