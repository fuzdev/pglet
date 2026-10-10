-- The one `reset_rest` production 18.6 parses and then **refuses**: `0A000 parameter
-- "transaction_isolation" cannot be reset`. pglet answered `42601` at the second
-- word, so it agreed on neither the phase nor the code; now the code is compared
-- against the oracle's.
-- @test
-- @expect-error: parameter "transaction_isolation" cannot be reset
RESET TRANSACTION ISOLATION LEVEL;
