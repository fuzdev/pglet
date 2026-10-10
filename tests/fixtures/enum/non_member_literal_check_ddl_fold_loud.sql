-- The same DDL door through a fold rather than an operator — the member list reaches the
-- stored-expression walk through the probe schema, which used to be built without one.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');

-- @test
-- @expect-error: zzz is not a member of mood
-- @expect-message
CREATE TABLE ck (id INT, m mood NOT NULL, CHECK (greatest(m, 'zzz') > 'sad'));
