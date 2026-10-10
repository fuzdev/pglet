-- `UPDATE t SET … FROM s WHERE … RETURNING s.col` over a **matching** row — the value
-- half of `update/returning_from_source_column.sql`, which pins the same spelling's
-- declared type over an empty result.
--
-- The source's `k` is `7` and the target carries no `k`, so a build that lost the source
-- from the namespace cannot answer at all; one that kept it but resolved against the
-- target would have to invent a value. Its bare twin — `RETURNING k`, one claimant — is
-- `returning_from_source_bare_column.sql`.
-- @setup
CREATE TABLE rsq_t (id INT, v TEXT);
CREATE TABLE rsq_s (id INT, sv TEXT, k INT);
INSERT INTO rsq_t VALUES (1, 'TARGET');
INSERT INTO rsq_s VALUES (1, 'SOURCE', 7);

-- @test
UPDATE rsq_t SET v = 'hit' FROM rsq_s s WHERE s.id = rsq_t.id RETURNING s.k, s.sv;
