-- The DDL-time half, and the one that **persisted**: the `CHECK` was recorded here and
-- refused there, so the accepted catalog dumped to SQL stock 18.6 refuses on restore —
-- `pg_constraint` empty on the far side with psql exiting `0`.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');

-- @test
-- @expect-error: zzz is not a member of mood
-- @expect-message
CREATE TABLE ck (id INT, m mood NOT NULL, CHECK (m > 'zzz'));
