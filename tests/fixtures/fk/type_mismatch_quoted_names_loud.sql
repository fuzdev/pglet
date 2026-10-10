-- The constraint and column names are printed as written, inside their quotes, with no
-- further quoting: `"My FK"`, `"A b"`.
-- @setup
CREATE TABLE ftp ("K" INT PRIMARY KEY);

-- @expect-error: 42804 under My FK
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE "Ftc" ("A b" TEXT CONSTRAINT "My FK" REFERENCES ftp);
