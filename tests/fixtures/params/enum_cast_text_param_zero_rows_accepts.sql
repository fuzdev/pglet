-- `$1::text::<enum>` — the parameter is typed `text` by its inner cast, so the conversion
-- to the enum is of a `text` value and runs at evaluation, as 18.6's does: over no row a
-- label the enum lacks is converted nowhere. (An **undeclared** `$1::<enum>` converts at
-- `Bind` instead, which the harness's client cannot drive — `tests/README.md` § `-- @param`.)
-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE cell (id INT);

-- @param $1 text secret
-- @test
SELECT ($1::text::cell_visibility)::text AS v FROM cell;
