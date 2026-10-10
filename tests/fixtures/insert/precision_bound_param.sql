-- A bound `$N` over the extended protocol — the spine's real binding path — reaches
-- the same store finalizer, so the column's precision rounds the bound value.
--
-- The literal is spelled RFC 3339 because that is what the harness binds a
-- `timestamptz` param with (`OffsetDateTime::parse(s, &Rfc3339)`); a space separator
-- or a bare `+00` offset makes the harness itself refuse the literal on both legs,
-- which is an agreement about nothing.
-- @setup
CREATE TABLE tpp (a timestamptz(3));
-- @param $1 timestamptz 2020-01-02T03:04:05.6789Z
-- @test
INSERT INTO tpp VALUES ($1) RETURNING a;
