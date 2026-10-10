-- A name the session does not hold is `26000` — `FetchPreparedStatement`'s
-- `throwError` arm. The protocol's own `Close('S')` reaches the *same* store and is
-- a silent no-op, the two differing only by `DropPreparedStatement`'s `showError`
-- flag, so this SQLSTATE is the whole observable difference between them.
-- @test
-- @expect-error: prepared statement "nosuch" does not exist
DEALLOCATE nosuch;
