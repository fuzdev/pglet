-- The recorded `CHECK` as 18.6 prints it — the conversion is one cast to the enum, not a
-- cast to `text`, so the definition names the type it converts to.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE mc (id INT, s TEXT CHECK (s::mood IS NOT NULL));

-- @test
SELECT pg_get_constraintdef(oid) AS def FROM pg_constraint WHERE conname = 'mc_s_check';
