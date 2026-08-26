local wezterm = require 'wezterm'
local act = wezterm.action
local config = wezterm.config_builder()

-- Platform
-- https://wezterm.org/config/lua/wezterm/target_triple.html
if wezterm.target_triple == 'x86_64-unknown-linux-gnu' then
  -- config.front_end = "Software" -- [OpenGL, WebGpu, Software] use Software if no GPU
elseif wezterm.target_triple == 'x86_64-pc-windows-msvc' then
  config.front_end = "WebGpu"
  config.default_domain = 'WSL:Ubuntu'
  -- tells wezterm the current cwd (for tabs) & command status
  -- uses OSC 7/133 sequences supported by most terminals & fails silently if wezterm is missing
  -- source this in shell config (eg ~/.bashrc)
  -- https://github.com/wezterm/wezterm/blob/main/assets/shell-integration/wezterm.sh
elseif wezterm.target_triple == 'aarch64-apple-darwin' then
  -- left Option for terminal shortcuts; right Option for accents
  -- https://wezterm.org/config/keyboard-concepts.html
  config.send_composed_key_when_left_alt_is_pressed = false
  config.send_composed_key_when_right_alt_is_pressed = true
end

-- Window
-- https://wezterm.org/config/appearance.html
config.initial_cols = 176 -- default 80
config.initial_rows = 44 -- default 24
config.window_padding = { left = 8, right = 8, top = 8, bottom = 8 } -- default 0
config.window_background_opacity = 0.80 -- default 1.0
config.window_decorations = "RESIZE" -- default "TITLE | RESIZE"
config.adjust_window_size_when_changing_font_size = false -- default true

-- Appearance
local font = 'AtkynsonMono'
local font_patterns = {
  wezterm.home_dir .. '/Library/Fonts/*' .. font .. '*.*',
  '/Library/Fonts/*' .. font .. '*.*',
  '/usr/share/fonts/' .. font .. '/*',
}
for _, pattern in ipairs(font_patterns) do
  if #wezterm.glob(pattern) > 0 then
    config.font = wezterm.font(font .. ' Nerd Font')
    break
  end
end
config.color_scheme = 'Catppuccin Mocha'
local palettes = {
  ['Catppuccin Mocha'] = {
    rosewater = '#f5e0dc',
    pink      = '#f5c2e7',
    mauve     = '#cba6f7',
    red       = '#f38ba8',
    peach     = '#fab387',
    yellow    = '#f9e2af',
    green     = '#a6e3a1',
    sky       = '#89dceb',
    blue      = '#89b4fa',
    lavender  = '#b4befe',
    text      = '#cdd6f4',
    subtext0  = '#a6adc8',
    overlay0  = '#313244',
    base      = '#1e1e2e',
    mantle    = '#181825',
    crust     = '#11111b'
  },
  ['Catppuccin Latte'] = {
    rosewater = '#dc8a78',
    pink      = '#ea76cb',
    mauve     = '#8839ef',
    red       = '#d20f39',
    peach     = '#fe640b',
    yellow    = '#df8e1d',
    green     = '#40a02b',
    sky       = '#04a5e5',
    blue      = '#1e66f5',
    lavender  = '#7287fd',
    text      = '#4c4f69',
    subtext0  = '#6c6f85',
    overlay0  = '#9ca0b0',
    base      = '#eff1f5',
    mantle    = '#e6e9ef',
    crust     = '#dce0e8'
  },
}
-- fallback to Catppuccin Mocha
local colors = palettes[config.color_scheme] or palettes['Catppuccin Mocha']

-- Cursor
-- https://wezterm.org/config/lua/config/default_cursor_style.html
config.default_cursor_style = 'BlinkingBlock'
config.cursor_blink_ease_in = 'Constant'
config.cursor_blink_ease_out = 'Constant'

-- Tab Bar
-- https://wezterm.org/config/appearance.html#tab-bar
config.use_fancy_tab_bar = false -- default true, we set our own tab bar style
config.tab_max_width = 32
config.colors = { tab_bar = { background = colors.base } }

-- Event Handlers
-- Nerd Font icon priority order: fa, md, oct, pl
wezterm.on('update-status', function(window, pane)
  local mode_text = "NORMAL"
  local mode_bg = colors.green
  local tab = pane:tab()
  if tab then
    for _, pane_info in ipairs(tab:panes_with_info()) do
      if pane_info.is_active and pane_info.is_zoomed then
        mode_text = "ZOOM"
        mode_bg = colors.peach
        break
      end
    end
  end
  if window:active_key_table() then
    mode_text = window:active_key_table():upper():gsub("_MODE", "") -- converts "tab_mode" to "TAB"
    mode_bg = colors.green
  end
  -- left status for mode
  window:set_left_status(wezterm.format {
    { Attribute = { Intensity = 'Bold' } },
    { Background = { Color = mode_bg } },
    { Foreground = { Color = colors.mantle } },
    { Text = '  ' .. string.format("%-6s", mode_text) .. ' ' }, -- nf-oct-terminal
    { Background = { Color = colors.base } },
    { Foreground = { Color = mode_bg } },
    { Text = '' }, -- nf-pl-left_hard_divider
  })
  -- right status for workspace
  local cwd = ""
  if pane:get_current_working_dir() then
    cwd = pane:get_current_working_dir().file_path:gsub("\\", "/") -- convert Win paths to Unix style
  end
  -- replace home with ~ (replaces /home/user on WSL as well)
  cwd = cwd:gsub(wezterm.home_dir:gsub("\\", "/"), "~"):gsub("^/home/[^/]+", "~")
  -- match last 2 folders in path (eg converts ~/code/a/ to code/a). If no match, fallback to cwd
  cwd = cwd:match("([^/]+/[^/]+)/?$") or cwd
  window:set_right_status(wezterm.format {
    { Foreground = { Color = colors.peach } },
    { Attribute = { Intensity = 'Bold' } },
    { Text = '  ' .. cwd .. ' ' }, -- nf-fa-folder
    { Foreground = { Color = colors.pink } },
    { Text = '' }, -- nf-fa-ellipsis_vertical
    { Foreground = { Color = colors.peach } },
    { Text = '  ' .. window:active_workspace() .. ' ' }, -- nf-fa-desktop
  })
end)

-- tab title formatting
-- https://wezterm.org/config/lua/window-events/format-tab-title.html
wezterm.on('format-tab-title', function(tab, tabs, panes, config, hover, max_width)
  local title = tab.active_pane.title -- title of active pane in tab
  -- use custom tab title if set (via tab:set_title())
  if tab.tab_title and #tab.tab_title > 0 then title = tab.tab_title end
  -- ensure title fits within max width
  title = wezterm.truncate_right(title, max_width - 4)
  local bg_color = colors.text
  if tab.is_active then
    bg_color = colors.green
  elseif hover then
    bg_color = colors.subtext0
  end
  return {
    { Background = { Color = bg_color } },
    { Foreground = { Color = colors.base } },
    { Text = '' },
    { Background = { Color = bg_color } },
    { Foreground = { Color = colors.mantle } },
    { Attribute = { Intensity = 'Bold' } },
    { Text = ' ' .. title .. ' ' },
    { Background = { Color = colors.base } },
    { Foreground = { Color = bg_color } },
    { Text = '' },
  }
end)

-- Key Bindings
-- https://wezterm.org/config/keys.html
-- https://zellij.dev/documentation/keybindings-modes
-- Keep WezTerm's platform-native defaults and add Zellij-style workspace controls.
config.keys = {
  -- direct pane actions
  { key = 'h', mods = 'ALT', action = act.ActivatePaneDirection 'Left' },
  { key = 'l', mods = 'ALT', action = act.ActivatePaneDirection 'Right' },
  { key = 'k', mods = 'ALT', action = act.ActivatePaneDirection 'Up' },
  { key = 'j', mods = 'ALT', action = act.ActivatePaneDirection 'Down' },
  { key = 'n', mods = 'ALT', action = act.SplitHorizontal { domain = 'CurrentPaneDomain' } },
  -- tab mode
  { key = 't', mods = 'CTRL', action = act.ActivateKeyTable {
    name = 'tab_mode', one_shot = false, replace_current = true,
  }},
  -- pane mode
  { key = 'p', mods = 'CTRL', action = act.ActivateKeyTable {
    name = 'pane_mode', one_shot = false, replace_current = true,
  } },
  -- resize mode
  { key = 'n', mods = 'CTRL', action = act.ActivateKeyTable {
    name = 'resize_mode', one_shot = false, replace_current = true,
  } },
  -- move mode
  { key = 'h', mods = 'CTRL', action = act.ActivateKeyTable {
    name = 'move_mode', one_shot = false, replace_current = true,
  }},
}

-- Key Tables
-- https://wezterm.org/config/key-tables.html
local function action_and_exit(action)
  return act.Multiple { action, act.PopKeyTable }
end

local key_tables = {
  tab_mode = {
    -- actions
    { key = 'n', action = action_and_exit(act.SpawnTab 'CurrentPaneDomain') },
    { key = 'x', action = action_and_exit(act.CloseCurrentTab { confirm = false }) },
    -- navigation
    { key = 'h', action = act.ActivateTabRelative(-1) },
    { key = 'LeftArrow', action = act.ActivateTabRelative(-1) },
    { key = 'j', action = act.ActivateTabRelative(1) },
    { key = 'DownArrow', action = act.ActivateTabRelative(1) },
    { key = 'k', action = act.ActivateTabRelative(-1) },
    { key = 'UpArrow', action = act.ActivateTabRelative(-1) },
    { key = 'l', action = act.ActivateTabRelative(1) },
    { key = 'RightArrow', action = act.ActivateTabRelative(1) },
    { key = '1', action = action_and_exit(act.ActivateTab(0)) },
    { key = '2', action = action_and_exit(act.ActivateTab(1)) },
    { key = '3', action = action_and_exit(act.ActivateTab(2)) },
    { key = '4', action = action_and_exit(act.ActivateTab(3)) },
    { key = '5', action = action_and_exit(act.ActivateTab(4)) },
    { key = '6', action = action_and_exit(act.ActivateTab(5)) },
    { key = '7', action = action_and_exit(act.ActivateTab(6)) },
    { key = '8', action = action_and_exit(act.ActivateTab(7)) },
    { key = '9', action = action_and_exit(act.ActivateTab(8)) },
    -- rename
    { key = 'r',
      action = act.PromptInputLine {
        description = 'Enter new name for tab',
        action = wezterm.action_callback(function(window, pane, line)
          if line then window:active_tab():set_title(line) end
          window:perform_action(act.PopKeyTable, pane)
        end),
      },
    },
    { key = 't', mods = 'CTRL', action = 'PopKeyTable' },
  },
  pane_mode = {
    -- actions
    { key = 'n', action = action_and_exit(act.SplitHorizontal { domain = 'CurrentPaneDomain' }) },
    { key = 'r', action = action_and_exit(act.SplitHorizontal { domain = 'CurrentPaneDomain' }) },
    { key = 'd', action = action_and_exit(act.SplitVertical { domain = 'CurrentPaneDomain' }) },
    { key = 'x', action = action_and_exit(act.CloseCurrentPane { confirm = false }) },
    { key = 'f', action = action_and_exit(act.TogglePaneZoomState) },
    -- navigation
    { key = 'h', action = act.ActivatePaneDirection 'Left' },
    { key = 'LeftArrow', action = act.ActivatePaneDirection 'Left' },
    { key = 'j', action = act.ActivatePaneDirection 'Down' },
    { key = 'DownArrow', action = act.ActivatePaneDirection 'Down' },
    { key = 'k', action = act.ActivatePaneDirection 'Up' },
    { key = 'UpArrow', action = act.ActivatePaneDirection 'Up' },
    { key = 'l', action = act.ActivatePaneDirection 'Right' },
    { key = 'RightArrow', action = act.ActivatePaneDirection 'Right' },
    { key = 'p', action = act.ActivatePaneDirection 'Next' }, -- next pane
    { key = 'p', mods = 'CTRL', action = 'PopKeyTable' },
  },
  resize_mode = {
    { key = 'h', action = act.AdjustPaneSize { 'Left', 5 } },
    { key = 'LeftArrow', action = act.AdjustPaneSize { 'Left', 5 } },
    { key = 'j', action = act.AdjustPaneSize { 'Down', 5 } },
    { key = 'DownArrow', action = act.AdjustPaneSize { 'Down', 5 } },
    { key = 'k', action = act.AdjustPaneSize { 'Up', 5 } },
    { key = 'UpArrow', action = act.AdjustPaneSize { 'Up', 5 } },
    { key = 'l', action = act.AdjustPaneSize { 'Right', 5 } },
    { key = 'RightArrow', action = act.AdjustPaneSize { 'Right', 5 } },
    { key = 'n', mods = 'CTRL', action = 'PopKeyTable' },
  },
  move_mode = {
    { key = 'n', action = act.RotatePanes 'Clockwise' },
    { key = 'p', action = act.RotatePanes 'CounterClockwise' },
    { key = 'h', mods = 'CTRL', action = 'PopKeyTable' },
  },
}

-- global exit keys for key tables
local common_keys = {
  { key = 'Escape', action = 'PopKeyTable' },
  { key = 'Enter', action = 'PopKeyTable' }
}
for _, tables in pairs(key_tables) do
  for _, key_obj in ipairs(common_keys) do table.insert(tables, key_obj) end
end
config.key_tables = key_tables -- set config.key_tables

-- Mouse Bindings
-- https://wezterm.org/config/mouse.html
config.hyperlink_rules = wezterm.default_hyperlink_rules()
config.mouse_bindings = {
  {
    event = { Up = { streak = 1, button = 'Left' } },
    mods = 'CTRL',
    action = act.OpenLinkAtMouseCursor
  },
  {
    event = { Down = { streak = 1, button = 'Right' } },
    action = act.PasteFrom 'Clipboard'
  },
}

return config
