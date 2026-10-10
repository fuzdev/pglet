-- The accept-side control for the cascade: a `CHECK` that does not convert to the enum is
-- no dependent and stays enforced.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE mk (n INT CHECK (n > 0), s TEXT CHECK (s::mood IS NOT NULL));

-- @test
-- @expect-error: the unrelated check survives the cascade
-- @expect-message
DROP TYPE mood CASCADE;
INSERT INTO mk VALUES (0, 'zzz');
