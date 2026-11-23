-- The shared form projected over a FROM answers once per row, as the exclusive one does
-- (`EDGES.md` row 250).
-- @setup
CREATE TABLE axls (a INT);
INSERT INTO axls VALUES (2), (1);
-- @test
SELECT a, pg_advisory_xact_lock_shared(a) AS k FROM axls WHERE a > 0;
