-- The other side of the same gate: a NULL the overload *does* accept still reaches
-- strictness and still yields NULL. A gate that rejected every typed NULL would break
-- this.
-- @setup
CREATE SEQUENCE svb_seq;
-- @test
SELECT setval('svb_seq', NULL::bigint);
