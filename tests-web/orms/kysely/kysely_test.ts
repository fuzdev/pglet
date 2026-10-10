/**
 * Kysely ORM tests for pglet compatibility.
 *
 * Self-contained: Automatically spawns pglet server if not running.
 *
 * Tests basic CRUD operations using Kysely ORM against pglet.
 * Run: deno task test:kysely
 *
 * Mirrors the Drizzle test structure for consistent coverage.
 */

import { assertEquals } from "@std/assert";
import { Kysely, type Migration, Migrator, PostgresDialect, sql } from "kysely";
import pg from "pg";
import type { Database, NewUser, NewPost } from "./types.ts";
import { PORTS } from "../../lib/ports.ts";
import { ensureServer } from "../../lib/server.ts";

const { Pool } = pg;

const CONFIG = {
  host: "127.0.0.1",
  port: PORTS.PGLET_TEST,
  user: "postgres",
  database: "postgres",
};

// Ensure server is running before tests
const cleanupServer = await ensureServer();

// Register cleanup
globalThis.addEventListener("unload", () => {
  cleanupServer();
});

/**
 * Helper to create a fresh Kysely connection.
 * Data persists across connections (shared state).
 */
async function withDb<T>(
  fn: (db: Kysely<Database>) => Promise<T>,
): Promise<T> {
  const pool = new Pool(CONFIG);
  const db = new Kysely<Database>({
    dialect: new PostgresDialect({ pool }),
  });
  try {
    return await fn(db);
  } finally {
    await db.destroy();
  }
}

/**
 * Setup tables using raw SQL.
 * Drops tables first since data persists across connections.
 */
async function setupTables(db: Kysely<Database>): Promise<void> {
  // Drop existing tables first (data persists across connections)
  await sql`DROP TABLE IF EXISTS posts CASCADE`.execute(db);
  await sql`DROP TABLE IF EXISTS users CASCADE`.execute(db);
  await sql`
    CREATE TABLE users (
      id INT PRIMARY KEY,
      name TEXT NOT NULL,
      email TEXT NOT NULL,
      active BOOL DEFAULT true,
      created_at TIMESTAMP
    )
  `.execute(db);
  await sql`
    CREATE TABLE posts (
      id INT PRIMARY KEY,
      title TEXT NOT NULL,
      content TEXT,
      author_id INT NOT NULL,
      published BOOL DEFAULT false
    )
  `.execute(db);
}

// =============================================================================
// INSERT Tests
// =============================================================================

Deno.test("kysely - insert single row", async () => {
  await withDb(async (db) => {
    await setupTables(db);

    await db.insertInto("users").values({
      id: 1,
      name: "Alice",
      email: "alice@example.com",
    }).execute();

    const result = await db.selectFrom("users").selectAll().execute();
    assertEquals(result.length, 1);
    assertEquals(result[0].name, "Alice");
    assertEquals(result[0].email, "alice@example.com");
  });
});

Deno.test("kysely - insert multiple rows", async () => {
  await withDb(async (db) => {
    await setupTables(db);

    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
      { id: 3, name: "Carol", email: "carol@example.com" },
    ]).execute();

    const result = await db.selectFrom("users").selectAll().execute();
    assertEquals(result.length, 3);
  });
});

// =============================================================================
// SELECT Tests
// =============================================================================

Deno.test("kysely - select all", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "alice@example.com", active: true },
      { id: 2, name: "Bob", email: "bob@example.com", active: false },
    ]).execute();

    const result = await db.selectFrom("users").selectAll().execute();
    assertEquals(result.length, 2);
  });
});

Deno.test("kysely - select with where eq", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
    ]).execute();

    const result = await db.selectFrom("users").selectAll().where("id", "=", 1).execute();
    assertEquals(result.length, 1);
    assertEquals(result[0].name, "Alice");
  });
});

Deno.test("kysely - select with where gt", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
      { id: 3, name: "Carol", email: "carol@example.com" },
    ]).execute();

    const result = await db.selectFrom("users").selectAll().where("id", ">", 1).execute();
    assertEquals(result.length, 2);
  });
});

Deno.test("kysely - select with and", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "alice@example.com", active: true },
      { id: 2, name: "Bob", email: "bob@example.com", active: false },
      { id: 3, name: "Carol", email: "carol@example.com", active: true },
    ]).execute();

    const result = await db
      .selectFrom("users")
      .selectAll()
      .where("active", "=", true)
      .where("id", ">", 1)
      .execute();
    assertEquals(result.length, 1);
    assertEquals(result[0].name, "Carol");
  });
});

Deno.test("kysely - select with or", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
      { id: 3, name: "Carol", email: "carol@example.com" },
    ]).execute();

    const result = await db
      .selectFrom("users")
      .selectAll()
      .where((eb) => eb.or([eb("id", "=", 1), eb("id", "=", 3)]))
      .execute();
    assertEquals(result.length, 2);
  });
});

Deno.test("kysely - select specific columns", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values({
      id: 1,
      name: "Alice",
      email: "alice@example.com",
    }).execute();

    const result = await db.selectFrom("users").select(["name", "email"]).execute();
    assertEquals(result.length, 1);
    assertEquals(result[0].name, "Alice");
    assertEquals(result[0].email, "alice@example.com");
    // @ts-expect-error id not selected
    assertEquals(result[0].id, undefined);
  });
});

Deno.test("kysely - select with order by asc", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 2, name: "Bob", email: "bob@example.com" },
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 3, name: "Carol", email: "carol@example.com" },
    ]).execute();

    const result = await db.selectFrom("users").selectAll().orderBy("name", "asc").execute();
    assertEquals(result[0].name, "Alice");
    assertEquals(result[1].name, "Bob");
    assertEquals(result[2].name, "Carol");
  });
});

Deno.test("kysely - select with order by desc", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
      { id: 3, name: "Carol", email: "carol@example.com" },
    ]).execute();

    const result = await db.selectFrom("users").selectAll().orderBy("id", "desc").execute();
    assertEquals(result[0].id, 3);
    assertEquals(result[1].id, 2);
    assertEquals(result[2].id, 1);
  });
});

Deno.test("kysely - select with limit", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
      { id: 3, name: "Carol", email: "carol@example.com" },
    ]).execute();

    const result = await db
      .selectFrom("users")
      .selectAll()
      .orderBy("id", "asc")
      .limit(2)
      .execute();
    assertEquals(result.length, 2);
    assertEquals(result[0].id, 1);
    assertEquals(result[1].id, 2);
  });
});

Deno.test("kysely - select with offset", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
      { id: 3, name: "Carol", email: "carol@example.com" },
    ]).execute();

    const result = await db
      .selectFrom("users")
      .selectAll()
      .orderBy("id", "asc")
      .limit(2)
      .offset(1)
      .execute();
    assertEquals(result.length, 2);
    assertEquals(result[0].id, 2);
    assertEquals(result[1].id, 3);
  });
});

// =============================================================================
// UPDATE Tests
// =============================================================================

Deno.test("kysely - update single field", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values({
      id: 1,
      name: "Alice",
      email: "alice@example.com",
    }).execute();

    await db.updateTable("users").set({ name: "Alicia" }).where("id", "=", 1).execute();

    const result = await db.selectFrom("users").selectAll().where("id", "=", 1).execute();
    assertEquals(result[0].name, "Alicia");
  });
});

Deno.test("kysely - update multiple fields", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values({
      id: 1,
      name: "Alice",
      email: "alice@example.com",
      active: true,
    }).execute();

    await db
      .updateTable("users")
      .set({ name: "Alicia", active: false })
      .where("id", "=", 1)
      .execute();

    const result = await db.selectFrom("users").selectAll().where("id", "=", 1).execute();
    assertEquals(result[0].name, "Alicia");
    assertEquals(result[0].active, false);
  });
});

Deno.test("kysely - update multiple rows", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "a@example.com", active: false },
      { id: 2, name: "Bob", email: "b@example.com", active: false },
      { id: 3, name: "Carol", email: "c@example.com", active: true },
    ]).execute();

    await db.updateTable("users").set({ active: true }).where("active", "=", false).execute();

    const result = await db.selectFrom("users").selectAll().execute();
    assertEquals(result.every((u) => u.active === true), true);
  });
});

// =============================================================================
// DELETE Tests
// =============================================================================

Deno.test("kysely - delete single row", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
    ]).execute();

    await db.deleteFrom("users").where("id", "=", 1).execute();

    const result = await db.selectFrom("users").selectAll().execute();
    assertEquals(result.length, 1);
    assertEquals(result[0].name, "Bob");
  });
});

Deno.test("kysely - delete multiple rows", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "a@example.com", active: false },
      { id: 2, name: "Bob", email: "b@example.com", active: false },
      { id: 3, name: "Carol", email: "c@example.com", active: true },
    ]).execute();

    await db.deleteFrom("users").where("active", "=", false).execute();

    const result = await db.selectFrom("users").selectAll().execute();
    assertEquals(result.length, 1);
    assertEquals(result[0].name, "Carol");
  });
});

Deno.test("kysely - delete all rows", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
    ]).execute();

    await db.deleteFrom("users").execute();

    const result = await db.selectFrom("users").selectAll().execute();
    assertEquals(result.length, 0);
  });
});

// =============================================================================
// Multi-table Tests
// =============================================================================

Deno.test("kysely - insert and query related tables", async () => {
  await withDb(async (db) => {
    await setupTables(db);

    await db.insertInto("users").values({ id: 1, name: "Alice", email: "a@ex.com" }).execute();
    await db.insertInto("posts").values([
      { id: 1, title: "First Post", content: "Hello", author_id: 1 },
      { id: 2, title: "Second Post", content: "World", author_id: 1 },
    ]).execute();

    const userResult = await db.selectFrom("users").selectAll().execute();
    assertEquals(userResult.length, 1);

    const postResult = await db
      .selectFrom("posts")
      .selectAll()
      .where("author_id", "=", 1)
      .execute();
    assertEquals(postResult.length, 2);
  });
});

// kysely's `Migrator` over the stock `PostgresDialect` on the native server, unstubbed: the
// introspector's `getTables` query and the migration lock (`pg_advisory_xact_lock`, the
// writer here) run as kysely issues them. The npm leg's twin is
// `tests-web/npm/kysely_migrator_test.ts`.
Deno.test("kysely - Migrator migrates, reruns as a no-op, rolls a failing migration back, migrates down", async () => {
  const pool = new Pool(CONFIG);
  // deno-lint-ignore no-explicit-any
  const db = new Kysely<any>({ dialect: new PostgresDialect({ pool }) });
  try {
    for (const t of ["kysely_migration", "kysely_migration_lock", "mig_person", "mig_pet"]) {
      await sql.raw(`DROP TABLE IF EXISTS ${t}`).execute(db);
    }
    const init: Migration = {
      up: async (db) => {
        await db.schema.createTable("mig_person").addColumn("id", "serial", (c) => c.primaryKey())
          .addColumn("name", "text").execute();
      },
      down: async (db) => {
        await db.schema.dropTable("mig_person").execute();
      },
    };
    const failing: Migration = {
      up: async (db) => {
        await db.schema.createTable("mig_pet").addColumn("id", "serial").execute();
        await sql`SELECT 1 / 0`.execute(db);
      },
    };
    let migrations: Record<string, Migration> = { "001_init": init };
    const migrator = new Migrator({ db, provider: { getMigrations: async () => migrations } });
    const names = async () =>
      (await db.introspection.getTables()).map((t) => t.name).filter((n) => n.startsWith("mig_")).sort();

    const first = await migrator.migrateToLatest();
    assertEquals(first.error, undefined);
    assertEquals(first.results?.map((r) => r.status), ["Success"]);
    assertEquals(await names(), ["mig_person"]);
    assertEquals((await migrator.migrateToLatest()).results, []);

    migrations = { "001_init": init, "002_failing": failing };
    const failed = await migrator.migrateToLatest();
    assertEquals((failed.error as { code?: string } | undefined)?.code, "22012");
    assertEquals(await names(), ["mig_person"]);

    migrations = { "001_init": init };
    const down = await migrator.migrateDown();
    assertEquals(down.error, undefined);
    assertEquals(await names(), []);
    for (const t of ["kysely_migration", "kysely_migration_lock"]) {
      await sql.raw(`DROP TABLE ${t}`).execute(db);
    }
  } finally {
    await db.destroy();
  }
});
