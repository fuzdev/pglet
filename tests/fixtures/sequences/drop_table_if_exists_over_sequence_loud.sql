-- `DROP TABLE IF EXISTS` skips a name **no relation holds**. A name a sequence holds
-- resolves, so it is the wrong *kind* — reported before the skip is ever reached
-- (`RemoveRelations` runs `DropErrorMsgWrongType` on a resolved name and
-- `DropErrorMsgNonExistent` only on an unresolved one). pglet skipped it and reported
-- **success**: the one silent member of this class, and one statement later
-- `CREATE TABLE dtie_seq` insisted the same name already existed.
-- @setup
CREATE SEQUENCE dtie_seq;
-- @test
DROP TABLE IF EXISTS dtie_seq;
-- @expect-error: a sequence is not a table, `IF EXISTS` notwithstanding (42809)
