-- The control that identifies the duplicate-label gate as a comparison of **string
-- literals** rather than of identifiers: a label is never folded, so `'A'` and `'a'` are
-- two members on both engines. Folding them would turn this ordinary type into a
-- refusal.
-- @setup
CREATE TYPE ctlc AS ENUM ('A', 'a', 'b');
CREATE TABLE ctlc_t (id INT, v ctlc);
INSERT INTO ctlc_t VALUES (1, 'a'), (2, 'A'), (3, 'b');
-- @test
SELECT id, v::text AS label FROM ctlc_t ORDER BY v, id;
