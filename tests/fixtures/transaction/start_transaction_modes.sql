-- @no-txn
-- The same list under the other spelling and with the optional commas written —
-- `transaction_mode_list` has both a comma'd and a juxtaposed production, so the
-- separator carries no meaning and both forms must reach the same block.
-- @setup
DROP TABLE IF EXISTS start_modes CASCADE;
CREATE TABLE start_modes (n INT);
START TRANSACTION ISOLATION LEVEL SERIALIZABLE, READ WRITE, NOT DEFERRABLE;
INSERT INTO start_modes VALUES (2);
COMMIT;

-- @test
SELECT n FROM start_modes;
