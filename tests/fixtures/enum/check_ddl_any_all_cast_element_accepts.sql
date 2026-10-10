-- The DDL-side over-refusal control: an element the parser leaves undecidable is not an
-- `unknown`, so the fold above must not reach it. 18.6 types `ARRAY['ok'::mood]` as
-- `mood[]` and records the constraint; the row the `INSERT` then lands is what says the
-- constraint was recorded and holds.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE ck (id INT, m mood NOT NULL, CHECK (m = ANY(ARRAY['ok'::mood, 'happy'::mood])));
INSERT INTO ck VALUES (1, 'ok'), (2, 'happy');

-- @test
SELECT id, m::text AS label FROM ck ORDER BY id;
