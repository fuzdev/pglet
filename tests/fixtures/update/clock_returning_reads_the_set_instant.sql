-- `RETURNING` reads the instant `SET` wrote: a `now()` in the returning list equals the
-- stamp the same statement stored, for every row.
-- @setup
CREATE TABLE cur (id INT, a TIMESTAMPTZ, s TIMESTAMPTZ);
INSERT INTO cur (id) VALUES (1), (2), (3);
-- @test
UPDATE cur SET a = now(), s = statement_timestamp()
RETURNING id, a = now() AS set_is_returning, a = transaction_timestamp() AS set_is_transaction,
          s = statement_timestamp() AS statement_is_returning, s <= clock_timestamp() AS clock_not_before;
