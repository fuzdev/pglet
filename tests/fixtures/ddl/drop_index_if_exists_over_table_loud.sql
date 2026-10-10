-- The second silent member of the class, and the mirror of
-- `drop_table_if_exists_over_index_loud`: the kind check reads the whole relation
-- namespace, so a **table** beats the `IF EXISTS` skip exactly as a sequence does.
-- pglet skipped and reported success here.
-- @setup
CREATE TABLE diiet_t (a INT);
-- @test
DROP INDEX IF EXISTS diiet_t;
-- @expect-error: a table is not an index, `IF EXISTS` notwithstanding (42809)
