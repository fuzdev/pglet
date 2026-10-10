-- Both overloads, beside an ordinary column: `(bigint)` over an `int8` literal (kysely's
-- migration-lock key), an `int4` and an `int2`, and `(int4, int4)`. Each is `void`.
-- @test
SELECT
  pg_advisory_xact_lock(3853314791062309107) AS kysely_key,
  pg_advisory_xact_lock(1) AS int4_key,
  pg_advisory_xact_lock(1::int2) AS int2_key,
  pg_advisory_xact_lock(1, 2) AS pair,
  2 AS beside;
