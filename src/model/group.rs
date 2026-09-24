use crate::model::channel::TdmsChannelData;
use crate::model::datatypes::PropertyValue;
use indexmap::IndexMap;

#[derive(PartialEq)]
pub struct TdmsGroupData {
    pub name: String,
    pub channels: IndexMap<String, TdmsChannelData>,
    pub properties: IndexMap<String, PropertyValue>,
}
