# grove 🌴

A featherweight CLI version control tool. ~900kb. You create a project, a list of files and folders on your machine, and you make saves of it, to be restored at will.

## Install

**Linux / macOS:**

```sh
curl -fsSL https://yourusername.github.io/grove/install.sh | sh
```

**Windows (PowerShell):**

```powershell
irm https://yourusername.github.io/grove/install.ps1 | iex
```

Or download a binary directly from the [releases page](https://github.com/YourUsername/grove/releases).

## saves
A backup of your files. 
Saves can be restored later.

### chapters
Saves will organize into chapters once you have 99+ in one project. Chapters are a folder in your directory, and a page in your UI.

### labyrinth
By default, all your selected folders and files are stored in the root of the backup folder when you make a save. Saves can alternatively be stored in labyrinth mode, where the original paths for every element are recreated inside the backup folder. 

## Screens

### Menu Screen

- **Create** - Create a new project
- **Grove Root** - Opens your Grove root directory, containing all projects
- **Open Project** - Open an existing project
- **Exit** - Quit Grove

#### Shortcuts

Entering a project name opens it. Entering `create` + a name will create a project with that name.

### Project Screen

- **Save** - Creates a backup of the files/folders listed in your project.
- **Files** - Add or delist folders and files from your project.
  
  **Files Screen** — Lists the files and folders in your project. You can add (filepicker) or delist any element.
- **Restore** - Restore one of your backups to their original location(s).
  
  **Saves Screen** — Lists all your saves with a date and time stamp.

> `chapters` will list your chapters for you to select which one you want to view.
> `convert` converts the selected save between flat and labyrinth storage.
> `delete` or del deletes the selected save(s) with a warning.
- **Grove** - Opens the directory for the current project, with all its backups.

#### Hidden Options

- `labyrinth` or `lab` toggles the storage of future saves between default and labyrinth mode. Indicated in the UI when on.

- `delete` deletes the project with a warning.

## License

MIT