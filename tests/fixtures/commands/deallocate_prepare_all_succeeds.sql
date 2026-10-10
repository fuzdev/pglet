-- `PREPARE` is a noise word (`deallocate.sgml`: "is ignored"), so this is the same
-- statement as the bare spelling and reports the same `DEALLOCATE ALL` tag — not
-- `DEALLOCATE`, since the tag keys on `DeallocateStmt.name == NULL` and not on
-- which of the four spellings was written.
-- @test
DEALLOCATE PREPARE ALL;
