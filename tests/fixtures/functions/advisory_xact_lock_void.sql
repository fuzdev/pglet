-- `pg_advisory_xact_lock(bigint)` returns `void`: declared OID 2278 on both legs, the
-- empty text in the text format and zero bytes in the binary one. Taken inside the
-- fixture's block, and taken again at once by the block that holds it.
-- @test
SELECT pg_advisory_xact_lock(7);
SELECT pg_advisory_xact_lock(7) AS again;
