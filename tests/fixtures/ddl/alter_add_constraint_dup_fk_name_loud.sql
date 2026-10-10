-- The same 42710 on the `ALTER TABLE … ADD CONSTRAINT … FOREIGN KEY` path, which had
-- no name check at all (only `ADD CONSTRAINT … CHECK` did).
-- @setup
CREATE TABLE aadf_p (id INT PRIMARY KEY);
CREATE TABLE aadf_ch (a INT, b INT);
ALTER TABLE aadf_ch ADD CONSTRAINT dup FOREIGN KEY (a) REFERENCES aadf_p(id);

-- @expect-error: constraint dup for relation aadf_ch already exists
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE aadf_ch ADD CONSTRAINT dup FOREIGN KEY (b) REFERENCES aadf_p(id);
