-- Postgres 18 registers no `min(uuid)` / `max(uuid)` — `pg_aggregate` carries no
-- uuid row, exactly as for `bool` and `jsonb` — so pglet rejects them the same way
-- rather than accepting on the guess that a later release adds them. A `uuid`
-- column is fully comparable and B-tree-indexable here; the accept-list is a
-- catalog fact, not a consequence of comparability.
-- @setup
CREATE TABLE agu (u UUID);
INSERT INTO agu VALUES ('11111111-1111-1111-1111-111111111111');
-- @test
-- @expect-error: function min(uuid) does not exist
SELECT min(u) FROM agu;
