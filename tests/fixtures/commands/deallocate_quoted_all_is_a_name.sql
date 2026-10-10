-- `ALL` is a reserved keyword, so `gram.y`'s `DEALLOCATE ALL` production can only
-- match the bare token: a quoted spelling is an IDENT and therefore a `name`. So
-- this drops the statement *named* `ALL` and, holding none, is `26000` — where an
-- implementation folding the quotes away would report a successful `DEALLOCATE ALL`
-- and silently drop every prepared statement the session had. Measured on 18.6 with
-- a real `PREPARE "ALL"`: it drops that one and leaves the others.
-- @test
-- @expect-error: prepared statement "ALL" does not exist
DEALLOCATE "ALL";
