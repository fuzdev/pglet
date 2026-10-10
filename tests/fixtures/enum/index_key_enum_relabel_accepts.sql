-- The accept-side control: a cast of a column that is already the enum is a relabel, no
-- function at all, so the index key and predicate are accepted — as is a member literal.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE ir (id INT, m mood);
INSERT INTO ir VALUES (1, 'ok'), (2, 'sad');

-- @test
CREATE INDEX ir_kx ON ir ((m::mood));
CREATE INDEX ir_px ON ir (id) WHERE m::mood = 'ok';
CREATE INDEX ir_lx ON ir (id) WHERE m = 'happy'::mood;
SELECT indexname FROM pg_indexes WHERE tablename = 'ir' ORDER BY indexname;
