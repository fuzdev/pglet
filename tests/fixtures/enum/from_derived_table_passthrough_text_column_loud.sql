-- The control on the accept fixtures beside it, and the one a **blanket** repair fails:
-- answering *undecidable* for every derived-table column — which falls the pair back to
-- the OID rule and accepts — makes this statement answer rows where 18.6 refuses. The
-- column here is a plain passthrough `text` one, so the descriptor could always decide
-- it; only a derivation that keeps deciding keeps this loud.
-- @expect-error: mood compared with a derived table's text column
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE w (id INT, m mood NOT NULL);
INSERT INTO w VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');
CREATE TABLE u (id INT, s TEXT NOT NULL);
INSERT INTO u VALUES (1, 'ok');

-- @test
SELECT w.id FROM w, (SELECT s AS z FROM u) v WHERE w.m = v.z;
