-- `IF EXISTS` INVERTS the positional rule rather than weakening it. Without it,
-- `DROP TABLE <missing>, <a sequence>` is `42P01` (the first name wins —
-- `drop_table_multi_positional_missing_first_loud.sql`). With it the first name is
-- skipped, so the later wrong kind is the first failure and the answer is `42809`.
-- Same statement, same order, different SQLSTATE — measured on 18.6.
-- @setup
CREATE SEQUENCE dpi_seq;

-- @expect-error: "dpi_seq" is not a table
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
DROP TABLE IF EXISTS dpi_nosuch, dpi_seq;
