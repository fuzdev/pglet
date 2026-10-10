-- An undeclared `$1` whose type an earlier occurrence fixed as `text`: 18.6 fixes a
-- parameter's type at its first coercion in parse-analysis order, so the later
-- `$1::<enum>` converts a `text` value, on the value — over no row a label the enum lacks
-- is converted nowhere. (The swapped spelling, the cast first, converts at `Bind` and is
-- refused over no row; 18.6 then reports the enum's own OID, which the harness's client
-- cannot bind — `tests/README.md` § `-- @param` — so that half is held off-corpus.)
-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE cell (id INT);
INSERT INTO cell VALUES (1);

-- @param $1 text secret
-- @test
SELECT $1::text AS a, ($1::cell_visibility)::text AS b FROM cell WHERE false;
