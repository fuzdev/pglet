-- `ADD COLUMN` with a constant default needs no rewrite, so the column's UNIQUE index is built as
-- its subcommand runs — before the foreign key is added — and the build's `23505` outranks the
-- key's `42804` (the per-row-volatile spelling is the other way round:
-- `add_column_type_mismatch_beats_rewrite_unique_build_loud`).
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);
CREATE TABLE ftc (z INT);
INSERT INTO ftc VALUES (1), (2);

-- @expect-error: 23505 could not create unique index ftc_b_key
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE ftc ADD COLUMN b TEXT DEFAULT 'q' UNIQUE REFERENCES ftp;
