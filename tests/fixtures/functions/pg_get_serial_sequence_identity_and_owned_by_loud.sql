-- pglet-stricter, and loud: a column owning more than one sequence is refused `0A000` —
-- an identity column's sequence and one attached after it (18.6 answers `public.t3_e_seq`). 18.6 answers the first `pg_depend` entry, creation order in every case measured, and
-- pglet keeps no creation order for ownership (`EDGES.md` row 252). Documentary: 18.6 answers,
-- so any pglet error passes; it holds the refusal against a fix that answers by name order.
-- @setup
CREATE TABLE t3 (e int GENERATED ALWAYS AS IDENTITY);
CREATE SEQUENCE a0 OWNED BY t3.e;
-- @test
-- @no-profile-lock: a SELECT with no FROM; no scan runs, so no index is selected on either profile
-- @expect-error: a column owning more than one sequence is refused here
SELECT pg_get_serial_sequence('t3', 'e');
