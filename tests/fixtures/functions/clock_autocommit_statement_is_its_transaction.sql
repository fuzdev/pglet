-- @no-txn
-- Outside a block a simple-protocol statement is its own transaction, which starts when
-- the statement does: `now()` and `statement_timestamp()` are one instant.
--
-- Text format only. Through the extended protocol 18.6 starts the transaction at one
-- message and restamps the statement at a later one, so the two differ there.
-- @text
-- @test
SELECT now() = statement_timestamp() AS transaction_is_statement,
       transaction_timestamp() = statement_timestamp() AS spelled_out,
       statement_timestamp() <= clock_timestamp() AS clock_not_before;
