-- The table-level `FOREIGN KEY (…) REFERENCES` door refuses an unimplementable pair as the
-- inline one does, under the same minted name.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);

-- @expect-error: 42804 text referencing integer
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (b TEXT, FOREIGN KEY (b) REFERENCES ftp (k));
