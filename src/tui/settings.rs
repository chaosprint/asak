use crate::tui::audio::{available_input_devices, available_output_devices};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsField {
    PlaybackDevice,
    RecordingDevice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsLevel {
    FieldSelect,
    DeviceSelect,
}

impl SettingsField {
    pub(crate) fn index(self) -> usize {
        match self {
            Self::PlaybackDevice => 0,
            Self::RecordingDevice => 1,
        }
    }

    pub(crate) fn next(self) -> Self {
        match self {
            Self::PlaybackDevice => Self::RecordingDevice,
            Self::RecordingDevice => Self::PlaybackDevice,
        }
    }

    pub(crate) fn previous(self) -> Self {
        self.next()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DeviceSettings {
    pub(crate) output_devices: Vec<String>,
    pub(crate) input_devices: Vec<String>,
    pub(crate) selected_output: usize,
    pub(crate) selected_input: usize,
    pub(crate) focused_field: SettingsField,
    pub(crate) level: SettingsLevel,
    pub(crate) status_message: Option<String>,
}

impl DeviceSettings {
    pub(crate) fn new() -> Self {
        let mut settings = Self {
            output_devices: Vec::new(),
            input_devices: Vec::new(),
            selected_output: 0,
            selected_input: 0,
            focused_field: SettingsField::PlaybackDevice,
            level: SettingsLevel::FieldSelect,
            status_message: None,
        };
        settings.refresh();
        settings
    }

    pub(crate) fn refresh(&mut self) {
        let selected_output_name = self.selected_output_name().map(str::to_owned);
        match available_output_devices() {
            Ok((devices, default_name)) => {
                let selection = select_index(
                    &devices,
                    selected_output_name.as_deref(),
                    default_name.as_deref(),
                );
                self.output_devices = devices;
                self.selected_output = selection;
                self.status_message = None;
            }
            Err(err) => self.status_message = Some(err.to_string()),
        }

        let selected_input_name = self.selected_input_name().map(str::to_owned);
        match available_input_devices() {
            Ok((devices, default_name)) => {
                let selection = select_index(
                    &devices,
                    selected_input_name.as_deref(),
                    default_name.as_deref(),
                );
                self.input_devices = devices;
                self.selected_input = selection;
            }
            Err(err) => self.status_message = Some(err.to_string()),
        }
    }

    pub(crate) fn focus_next(&mut self) {
        self.focused_field = self.focused_field.next();
    }

    pub(crate) fn focus_previous(&mut self) {
        self.focused_field = self.focused_field.previous();
    }

    pub(crate) fn enter_field(&mut self) {
        self.level = SettingsLevel::DeviceSelect;
    }

    pub(crate) fn leave_field(&mut self) {
        self.level = SettingsLevel::FieldSelect;
    }

    pub(crate) fn is_selecting_device(&self) -> bool {
        self.level == SettingsLevel::DeviceSelect
    }

    pub(crate) fn select_next_device(&mut self) {
        self.cycle_current_forward();
    }

    pub(crate) fn select_previous_device(&mut self) {
        self.cycle_current_backward();
    }

    pub(crate) fn cycle_current_forward(&mut self) {
        match self.focused_field {
            SettingsField::PlaybackDevice => {
                cycle_index(&mut self.selected_output, self.output_devices.len(), true)
            }
            SettingsField::RecordingDevice => {
                cycle_index(&mut self.selected_input, self.input_devices.len(), true)
            }
        }
    }

    pub(crate) fn cycle_current_backward(&mut self) {
        match self.focused_field {
            SettingsField::PlaybackDevice => {
                cycle_index(&mut self.selected_output, self.output_devices.len(), false)
            }
            SettingsField::RecordingDevice => {
                cycle_index(&mut self.selected_input, self.input_devices.len(), false)
            }
        }
    }

    pub(crate) fn selected_output_name(&self) -> Option<&str> {
        self.output_devices
            .get(self.selected_output)
            .map(String::as_str)
    }

    pub(crate) fn selected_input_name(&self) -> Option<&str> {
        self.input_devices
            .get(self.selected_input)
            .map(String::as_str)
    }

    pub(crate) fn active_selected_index(&self) -> Option<usize> {
        match self.focused_field {
            SettingsField::PlaybackDevice if !self.output_devices.is_empty() => {
                Some(self.selected_output)
            }
            SettingsField::RecordingDevice if !self.input_devices.is_empty() => {
                Some(self.selected_input)
            }
            _ => None,
        }
    }
}

fn select_index(
    devices: &[String],
    selected_name: Option<&str>,
    default_name: Option<&str>,
) -> usize {
    if devices.is_empty() {
        return 0;
    }

    if let Some(selected_name) = selected_name {
        if let Some(index) = devices.iter().position(|device| device == selected_name) {
            return index;
        }
    }

    if let Some(default_name) = default_name {
        if let Some(index) = devices.iter().position(|device| device == default_name) {
            return index;
        }
    }

    0
}

fn cycle_index(index: &mut usize, len: usize, forward: bool) {
    if len == 0 {
        *index = 0;
        return;
    }

    *index = if forward {
        (*index + 1) % len
    } else if *index == 0 {
        len - 1
    } else {
        *index - 1
    };
}
