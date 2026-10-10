-- The bare-name half of the same rule: a name the target does not carry is `42703`,
-- again independent of the row count. Zero rows deliberately — with a matching row the
-- executor's per-row resolver already raised this, so a populated fixture would pass
-- against the pre-check engine and pin nothing.
-- @setup
CREATE TABLE rub_t (id INT, b BIGINT);
INSERT INTO rub_t VALUES (1, 111);

-- @test
-- @expect-error: column "nosuchcol" does not exist (42703)
DELETE FROM rub_t WHERE id = 999 RETURNING nosuchcol + 0;
