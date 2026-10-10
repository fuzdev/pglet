-- The same classifier under the **other** grammar mode. A column constraint's `DEFAULT`
-- takes `gram.y`'s restricted `b_expr` and `ALTER COLUMN … SET DEFAULT` takes full
-- `a_expr`, which is a real split in `parser::parse_column_default` — so the two routes
-- into the classifier are worth one fixture each, where `ALTER TABLE … ADD COLUMN`
-- (measured `42809` on both engines here too) is the same call site under the same mode
-- as `CREATE TABLE` and is not.
-- @setup
CREATE SEQUENCE sq_nsd_seq;
CREATE TABLE sq_nsd (a BIGINT);
-- @test
-- @expect-error: DISTINCT specified, but nextval is not an aggregate function
ALTER TABLE sq_nsd ALTER COLUMN a SET DEFAULT nextval(DISTINCT 'sq_nsd_seq');
