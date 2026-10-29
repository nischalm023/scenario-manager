CREATE TABLE "map_bot" (
	"project_id" text NOT NULL,
	"map_id" text NOT NULL,
	"type" text NOT NULL,
	"name" text NOT NULL,
	"image_url" text DEFAULT '' NOT NULL,
	"count" integer NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "map_bot_project_id_map_id_type_pk" PRIMARY KEY("project_id","map_id","type")
);
