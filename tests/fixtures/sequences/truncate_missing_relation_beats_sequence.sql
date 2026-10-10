-- `ExecuteTruncate` walks `stmt->relations` in written order, so the *first* named
-- relation decides: a missing one ahead of a sequence is 42P01, and the swap
-- (`TRUNCATE <a sequence>, <missing>`) is 42809. A kind check hoisted out of the walk
-- would answer 42809 both ways.
-- @setup
CREATE SEQUENCE trm_seq;
-- @test
TRUNCATE trm_nothing, trm_seq;
-- @expect-error: the first named relation is missing (42P01)
