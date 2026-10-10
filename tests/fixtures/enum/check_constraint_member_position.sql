-- **The divergence writes.** A `CHECK (m < 'ok')` is a stored expression re-evaluated on
-- every write, so the member order decides which rows land: 18.6 keeps `sad` and refuses
-- `happy`, and a label-byte comparison keeps and refuses *exactly the opposite* pair.
--
-- Asserted as a value fixture rather than an `@expect-error` one, deliberately: an
-- `@expect-error` fixture's `@setup` failure is reported as its whole result, so a
-- broken engine that refused the wrong row would score green off the wrong statement
-- (`tests/README.md` § Directives). Here the setup runs to completion and the surviving
-- row is the assertion.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE c (id INT, m mood, CHECK (m < 'ok'));
INSERT INTO c VALUES (1, 'sad');

-- @test
SELECT id, m::text AS label FROM c ORDER BY id;
