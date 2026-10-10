-- A failed `DO` puts back the counter cells its statements replaced before its error
-- returns — and a caught outer `DO` is where that is readable, since its handler runs
-- after the inner error. Over three draws, the first `DO`'s inner body restarts the
-- sequence and fails; the outer handler's draw reads the old cell, `4` on 18.6 (without
-- the inner undo it drew from the restarted cell, `100`). In the second, the inner body
-- restarts and fails, its handler draws `5` from the restored cell, restarts again and
-- fails too: the inner undo restores the cell the **handler** replaced, as the handler's
-- own draw left it, so the outer handler draws `6` on 18.6. Restoring the cell the body
-- replaced instead — the swap read before the body's own undo — answers `5`, and no inner
-- undo at all leaves the handler's restart, `200`.
--
-- The catalog, row and index-spec halves of a failed `DO`'s undo are not readable this
-- way — the outer undo covers them too — so their guards are the Rust tests
-- `EDGES.md` row 249 names.
-- @setup
CREATE SEQUENCE dfi_s;
CREATE TABLE dfi_lg (v BIGINT);
CREATE TABLE dfi_scratch (v BIGINT);
INSERT INTO dfi_lg VALUES (nextval('dfi_s')), (nextval('dfi_s')), (nextval('dfi_s'));
DO $o$ BEGIN
    DO $i$ BEGIN ALTER SEQUENCE dfi_s RESTART WITH 100; INSERT INTO dfi_nosuch VALUES (1); END $i$;
EXCEPTION WHEN OTHERS THEN
    INSERT INTO dfi_lg VALUES (nextval('dfi_s'));
END $o$;
DO $o$ BEGIN
    DO $i$ BEGIN
        ALTER SEQUENCE dfi_s RESTART WITH 100;
        INSERT INTO dfi_nosuch VALUES (1);
    EXCEPTION WHEN OTHERS THEN
        INSERT INTO dfi_scratch VALUES (nextval('dfi_s'));
        ALTER SEQUENCE dfi_s RESTART WITH 200;
        INSERT INTO dfi_nosuch VALUES (2);
    END $i$;
EXCEPTION WHEN OTHERS THEN
    INSERT INTO dfi_lg VALUES (nextval('dfi_s'));
END $o$;
-- @test
SELECT v FROM dfi_lg ORDER BY v;
