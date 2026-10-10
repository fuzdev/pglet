-- The same refusal with no `FROM`: a select list that is nothing but a null test over
-- the parameter. (`undetermined_param_bare_select_entry.sql` is the control: a bare
-- select-list entry resolves to `text`.)
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text NULL
-- @test
-- @expect-error: 18.6 leaves the parameter untyped: 42P18
-- @expect-message
SELECT $1 IS NOT NULL AS b;
