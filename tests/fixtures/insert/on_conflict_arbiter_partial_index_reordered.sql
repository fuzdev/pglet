-- The two halves of the match are independent: the predicate still has to be restated and
-- is compared structurally, while the key parts are compared as a set — so a partial
-- index is matched by a reordered inference list carrying its predicate.
-- @setup
CREATE TABLE oap (a INT, b INT);
CREATE UNIQUE INDEX oap_ab ON oap (a, b) WHERE b > 0;
INSERT INTO oap VALUES (1, 1);
INSERT INTO oap VALUES (1, 1) ON CONFLICT (b, a) WHERE b > 0 DO NOTHING;

-- @test
SELECT a, b FROM oap ORDER BY a;
